//! PostgreSQL-backed skill metadata cache.
//!
//! Provides fast startup by caching parsed skill metadata in the database.
//! Filesystem remains the source of truth; the cache is updated on hash mismatch.

use std::{collections::HashMap, path::PathBuf};

use snafu::ResultExt;
use sqlx::PgPool;

use crate::{
    error::{InvalidInputSnafu, Result, SqlxSnafu},
    types::{SkillMetadata, SkillSource},
};

/// PostgreSQL-backed skill cache (backing store, not a SkillRegistry).
pub struct PgSkillCache {
    pool: PgPool,
}

/// Cached skill with hash for change detection.
#[derive(Debug, Clone)]
pub struct CachedSkill {
    pub metadata: SkillMetadata,
    pub content_hash: String,
}

impl PgSkillCache {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Load all cached skill metadata from the database.
    pub async fn load_all(&self) -> Result<HashMap<String, CachedSkill>> {
        let rows = sqlx::query_as::<_, rara_model::skill::SkillCache>(
            "SELECT * FROM skill_cache ORDER BY name",
        )
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

        let source_i16 = source_to_i16(meta.source.as_ref());

        sqlx::query(
            r#"INSERT INTO skill_cache
               (name, description, homepage, license, compatibility, allowed_tools,
                dockerfile, requires, path, source, content_hash, cached_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, NOW())
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
                 cached_at     = NOW()
            "#,
        )
        .bind(&meta.name)
        .bind(&meta.description)
        .bind(&meta.homepage)
        .bind(&meta.license)
        .bind(&meta.compatibility)
        .bind(&meta.allowed_tools)
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
        sqlx::query("DELETE FROM skill_cache WHERE name = $1")
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

// ── Source <-> i16 mapping ──────────────────────────────────────────────────

fn source_to_i16(source: Option<&SkillSource>) -> i16 {
    match source {
        Some(SkillSource::Project) => 0,
        Some(SkillSource::Personal) => 1,
        Some(SkillSource::Plugin) => 2,
        Some(SkillSource::Registry) => 3,
        None => -1,
    }
}

fn source_from_i16(value: i16) -> Option<SkillSource> {
    match value {
        0 => Some(SkillSource::Project),
        1 => Some(SkillSource::Personal),
        2 => Some(SkillSource::Plugin),
        3 => Some(SkillSource::Registry),
        _ => None,
    }
}

impl CachedSkill {
    fn from_db_row(row: rara_model::skill::SkillCache) -> Result<Self> {
        let requires = serde_json::from_value(row.requires).map_err(|e| {
            InvalidInputSnafu {
                message: format!("failed to deserialize requires: {e}"),
            }
            .build()
        })?;

        Ok(Self {
            metadata: SkillMetadata {
                name: row.name,
                description: row.description,
                homepage: row.homepage,
                license: row.license,
                compatibility: row.compatibility,
                allowed_tools: row.allowed_tools,
                dockerfile: row.dockerfile,
                requires,
                path: PathBuf::from(row.path),
                source: source_from_i16(row.source),
            },
            content_hash: row.content_hash,
        })
    }
}
