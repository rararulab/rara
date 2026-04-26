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

use diesel::{
    ExpressionMethods, QueryDsl, Queryable, Selectable, SelectableHelper, sql_types::Text,
    upsert::excluded,
};
use diesel_async::RunQueryDsl;
use rara_model::schema::skill_cache;
use snafu::ResultExt;
use yunara_store::diesel_pool::DieselSqlitePools;

use crate::{
    error::{DieselPoolSnafu, DieselSnafu, InvalidInputSnafu, Result},
    types::{SkillMetadata, SkillSource},
};

// ---------------------------------------------------------------------------
// DB row type
// ---------------------------------------------------------------------------

/// Cached skill metadata row from `skill_cache` table.
#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = skill_cache)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub(crate) struct SkillCacheRow {
    // `name` is the primary key — reported as `Nullable<Text>` by
    // `diesel print-schema` on SQLite but always present in practice.
    pub name:          Option<String>,
    pub description:   String,
    pub homepage:      Option<String>,
    pub license:       Option<String>,
    pub compatibility: Option<String>,
    pub allowed_tools: String,
    pub dockerfile:    Option<String>,
    pub requires:      String,
    pub path:          String,
    pub source:        i32,
    pub content_hash:  String,
    #[allow(dead_code)]
    pub cached_at:     String,
}

/// SQLite-backed skill cache (backing store, not a SkillRegistry).
pub struct SqliteSkillCache {
    pools: DieselSqlitePools,
}

/// Cached skill with hash for change detection.
#[derive(Debug, Clone)]
pub struct CachedSkill {
    pub metadata:     SkillMetadata,
    pub content_hash: String,
}

impl SqliteSkillCache {
    pub fn new(pools: DieselSqlitePools) -> Self { Self { pools } }

    /// Load all cached skill metadata from the database.
    pub async fn load_all(&self) -> Result<HashMap<String, CachedSkill>> {
        let mut conn = self.pools.reader.get().await.context(DieselPoolSnafu)?;
        let rows: Vec<SkillCacheRow> = skill_cache::table
            .select(SkillCacheRow::as_select())
            .order(skill_cache::name.asc())
            .load(&mut *conn)
            .await
            .context(DieselSnafu)?;

        let mut map = HashMap::with_capacity(rows.len());
        for row in rows {
            let cached = CachedSkill::from_db_row(row)?;
            map.insert(cached.metadata.name.clone(), cached);
        }
        Ok(map)
    }

    /// Upsert a skill into the cache.
    pub async fn upsert(&self, meta: &SkillMetadata, hash: &str) -> Result<()> {
        let requires_json = serde_json::to_string(&meta.requires).map_err(|e| {
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

        let source_i32: i32 = meta.source.map(|s| s as u8 as i32).unwrap_or(-1);
        let path_str = meta.path.to_string_lossy().into_owned();

        // SQLite `datetime('now')` has no cross-backend DSL — emit via the
        // sanctioned `sql::<Text>` escape hatch per
        // docs/guides/db-diesel-migration.md.
        let now_expr = diesel::dsl::sql::<Text>("datetime('now')");

        let mut conn = self.pools.writer.get().await.context(DieselPoolSnafu)?;
        diesel::insert_into(skill_cache::table)
            .values((
                skill_cache::name.eq(&meta.name),
                skill_cache::description.eq(&meta.description),
                skill_cache::homepage.eq(&meta.homepage),
                skill_cache::license.eq(&meta.license),
                skill_cache::compatibility.eq(&meta.compatibility),
                skill_cache::allowed_tools.eq(&allowed_tools_json),
                skill_cache::dockerfile.eq(&meta.dockerfile),
                skill_cache::requires.eq(&requires_json),
                skill_cache::path.eq(&path_str),
                skill_cache::source.eq(source_i32),
                skill_cache::content_hash.eq(hash),
                skill_cache::cached_at.eq(now_expr.clone()),
            ))
            .on_conflict(skill_cache::name)
            .do_update()
            .set((
                skill_cache::description.eq(excluded(skill_cache::description)),
                skill_cache::homepage.eq(excluded(skill_cache::homepage)),
                skill_cache::license.eq(excluded(skill_cache::license)),
                skill_cache::compatibility.eq(excluded(skill_cache::compatibility)),
                skill_cache::allowed_tools.eq(excluded(skill_cache::allowed_tools)),
                skill_cache::dockerfile.eq(excluded(skill_cache::dockerfile)),
                skill_cache::requires.eq(excluded(skill_cache::requires)),
                skill_cache::path.eq(excluded(skill_cache::path)),
                skill_cache::source.eq(excluded(skill_cache::source)),
                skill_cache::content_hash.eq(excluded(skill_cache::content_hash)),
                skill_cache::cached_at.eq(now_expr),
            ))
            .execute(&mut *conn)
            .await
            .context(DieselSnafu)?;

        Ok(())
    }

    /// Remove a skill from the cache by name.
    pub async fn remove(&self, name: &str) -> Result<()> {
        let mut conn = self.pools.writer.get().await.context(DieselPoolSnafu)?;
        diesel::delete(skill_cache::table.filter(skill_cache::name.eq(name)))
            .execute(&mut *conn)
            .await
            .context(DieselSnafu)?;
        Ok(())
    }

    /// Remove all skills from the cache.
    pub async fn clear(&self) -> Result<()> {
        let mut conn = self.pools.writer.get().await.context(DieselPoolSnafu)?;
        diesel::delete(skill_cache::table)
            .execute(&mut *conn)
            .await
            .context(DieselSnafu)?;
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
pub fn spawn_background_sync(
    pools: DieselSqlitePools,
    registry: crate::registry::InMemoryRegistry,
) {
    use std::collections::HashSet;

    use tracing::{info, warn};

    use crate::discover::{FsSkillDiscoverer, SkillDiscoverer};

    tokio::spawn(async move {
        let cache = SqliteSkillCache::new(pools);
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
        let requires = serde_json::from_str(&row.requires).map_err(|e| {
            InvalidInputSnafu {
                message: format!("failed to deserialize requires: {e}"),
            }
            .build()
        })?;

        let allowed_tools: Vec<String> = serde_json::from_str(&row.allowed_tools).map_err(|e| {
            InvalidInputSnafu {
                message: format!("failed to deserialize allowed_tools: {e}"),
            }
            .build()
        })?;

        let name = row.name.ok_or_else(|| {
            InvalidInputSnafu {
                message: "skill_cache row missing primary-key `name`".to_owned(),
            }
            .build()
        })?;

        Ok(Self {
            metadata:     SkillMetadata {
                name,
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
