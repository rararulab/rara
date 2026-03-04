// Copyright 2025 Rararulab
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! SQLite-backed skill metadata cache.
//!
//! Provides fast startup by caching parsed skill metadata in the database.
//! Filesystem remains the source of truth; the cache is updated on hash
//! mismatch.

use std::{collections::HashMap, path::PathBuf};

use chrono::{DateTime, Utc};
use snafu::ResultExt;
use sqlx::SqlitePool;

use crate::{
    error::{InvalidInputSnafu, Result, SqlxSnafu},
    types::{SkillMetadata, SkillSource},
};

// ---------------------------------------------------------------------------
// DB row type (sqlx::FromRow)
// ---------------------------------------------------------------------------

/// Cached skill metadata row from `skill_cache` table.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct SkillCacheRow {
    pub name:          String,
    pub description:   String,
    pub homepage:      Option<String>,
    pub license:       Option<String>,
    pub compatibility: Option<String>,
    pub allowed_tools: String,
    pub dockerfile:    Option<String>,
    pub requires:      serde_json::Value,
    pub path:          String,
    pub source:        i16,
    pub content_hash:  String,
    pub cached_at:     DateTime<Utc>,
}

/// SQLite-backed skill cache (backing store, not a SkillRegistry).
pub struct SqliteSkillCache {
    pool: SqlitePool,
}

/// Cached skill with hash for change detection.
#[derive(Debug, Clone)]
pub struct CachedSkill {
    pub metadata:     SkillMetadata,
    pub content_hash: String,
}

impl SqliteSkillCache {
    pub fn new(pool: SqlitePool) -> Self { Self { pool } }

    /// Load all cached skill metadata from the database.
    pub async fn load_all(&self) -> Result<HashMap<String, CachedSkill>> {
        let rows = sqlx::query_as::<_, SkillCacheRow>("SELECT * FROM skill_cache ORDER BY name")
            .fetch_all(&self.pool)
            .await
            .context(SqlxSnafu)?;

        let mut map = HashMap::with_capacity(rows.len());
        for row in rows {
            let cached = CachedSkill::from_db_row(row)?;
            map.insert(cached.metadata.name.clone(), cached);
        }
        Ok(map)
    }

    /// Upsert a skill into the cache.
    pub async fn upsert(&self, meta: &SkillMetadata, hash: &str) -> Result<()> {
        let requires_json = serde_json::to_value(&meta.requires).map_err(|e| {
            InvalidInputSnafu {
                message: format!("failed to serialize requires: {e}"),
            }
            .build()
        })?;

        let allowed_tools_json = serde_json::to_string(&meta.allowed_tools).map_err(|e| {
            InvalidInputSnafu {
                message: format!("failed to serialize allowed_tools: {e}"),
            }
            .build()
        })?;

        let source_i16: i16 = meta.source.map(|s| s as u8 as i16).unwrap_or(-1);

        sqlx::query(
            r#"INSERT INTO skill_cache
               (name, description, homepage, license, compatibility, allowed_tools,
                dockerfile, requires, path, source, content_hash, cached_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, datetime('now'))
               ON CONFLICT (name) DO UPDATE SET
                 description   = EXCLUDED.description,
                 homepage      = EXCLUDED.homepage,
                 license       = EXCLUDED.license,
                 compatibility = EXCLUDED.compatibility,
                 allowed_tools = EXCLUDED.allowed_tools,
                 dockerfile    = EXCLUDED.dockerfile,
                 requires      = EXCLUDED.requires,
                 path          = EXCLUDED.path,
                 source        = EXCLUDED.source,
                 content_hash  = EXCLUDED.content_hash,
                 cached_at     = datetime('now')
            "#,
        )
        .bind(&meta.name)
        .bind(&meta.description)
        .bind(&meta.homepage)
        .bind(&meta.license)
        .bind(&meta.compatibility)
        .bind(&allowed_tools_json)
        .bind(&meta.dockerfile)
        .bind(&requires_json)
        .bind(meta.path.to_string_lossy().as_ref())
        .bind(source_i16)
        .bind(hash)
        .execute(&self.pool)
        .await
        .context(SqlxSnafu)?;

        Ok(())
    }

    /// Remove a skill from the cache by name.
    pub async fn remove(&self, name: &str) -> Result<()> {
        sqlx::query("DELETE FROM skill_cache WHERE name = ?1")
            .bind(name)
            .execute(&self.pool)
            .await
            .context(SqlxSnafu)?;
        Ok(())
    }

    /// Remove all skills from the cache.
    pub async fn clear(&self) -> Result<()> {
        sqlx::query("DELETE FROM skill_cache")
            .execute(&self.pool)
            .await
            .context(SqlxSnafu)?;
        Ok(())
    }
}

// ── Background sync ─────────────────────────────────────────────────────────

/// Spawn a background task that populates `registry` from the SQLite cache,
/// then incrementally syncs with the filesystem using content hashing.
///
/// 1. **Phase 1** — load cached metadata from DB → fill registry (fast).
/// 2. **Phase 2** — FS scan + SHA-256 hash comparison → upsert changed skills.
/// 3. **Phase 3** — garbage-collect stale cache entries no longer on disk.
pub fn spawn_background_sync(pool: SqlitePool, registry: crate::registry::InMemoryRegistry) {
    use std::collections::HashSet;

    use tracing::{info, warn};

    use crate::discover::{FsSkillDiscoverer, SkillDiscoverer};

    tokio::spawn(async move {
        let cache = SqliteSkillCache::new(pool);
        let discoverer = FsSkillDiscoverer::new(FsSkillDiscoverer::default_paths());

        // Phase 1: load from SQLite cache (fast startup)
        let cached = match cache.load_all().await {
            Ok(c) => {
                let count = c.len();
                if count > 0 {
                    for cached_skill in c.values() {
                        registry.insert(cached_skill.metadata.clone());
                    }
                    info!(count, "Skills loaded from cache");
                }
                c
            }
            Err(e) => {
                warn!(error = %e, "Failed to load skill cache, falling back to FS");
                HashMap::new()
            }
        };

        // Phase 2: incremental FS sync with hash comparison
        match discoverer.discover().await {
            Ok(discovered) => {
                let mut added = 0u32;
                let mut changed = 0u32;
                let mut unchanged = 0u32;
                let mut seen_names = HashSet::new();

                for meta in &discovered {
                    seen_names.insert(meta.name.clone());
                    let skill_md = meta.path.join("SKILL.md");
                    let current_hash = match crate::hash::file_hash(&skill_md) {
                        Ok(h) => h,
                        Err(e) => {
                            warn!(skill = %meta.name, error = %e, "Failed to hash SKILL.md, skipping");
                            continue;
                        }
                    };

                    let needs_update = cached
                        .get(&meta.name)
                        .map(|c| c.content_hash != current_hash)
                        .unwrap_or(true);

                    if needs_update {
                        if cached.contains_key(&meta.name) {
                            changed += 1;
                        } else {
                            added += 1;
                        }
                        registry.insert(meta.clone());
                        if let Err(e) = cache.upsert(meta, &current_hash).await {
                            warn!(skill = %meta.name, error = %e, "Failed to update cache");
                        }
                    } else {
                        unchanged += 1;
                    }
                }

                // Phase 3: garbage-collect stale cache entries
                let mut removed = 0u32;
                for name in cached.keys() {
                    if !seen_names.contains(name) {
                        removed += 1;
                        registry.remove(name);
                        if let Err(e) = cache.remove(name).await {
                            warn!(skill = %name, error = %e, "Failed to remove stale cache entry");
                        }
                    }
                }

                let total = registry.list_all().len();
                info!(
                    total,
                    added, changed, unchanged, removed, "Skill registry synced"
                );
            }
            Err(e) => {
                warn!(error = %e, "Background skill discovery failed");
            }
        }
    });
}

impl CachedSkill {
    fn from_db_row(row: SkillCacheRow) -> Result<Self> {
        let requires = serde_json::from_value(row.requires).map_err(|e| {
            InvalidInputSnafu {
                message: format!("failed to deserialize requires: {e}"),
            }
            .build()
        })?;

        let allowed_tools: Vec<String> =
            serde_json::from_str(&row.allowed_tools).map_err(|e| {
                InvalidInputSnafu {
                    message: format!("failed to deserialize allowed_tools: {e}"),
                }
                .build()
            })?;

        Ok(Self {
            metadata:     SkillMetadata {
                name: row.name,
                description: row.description,
                homepage: row.homepage,
                license: row.license,
                compatibility: row.compatibility,
                allowed_tools,
                dockerfile: row.dockerfile,
                requires,
                path: PathBuf::from(row.path),
                source: SkillSource::from_repr(row.source as u8),
            },
            content_hash: row.content_hash,
        })
    }
}
