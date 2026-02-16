use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

/// Cached skill metadata row from `skill_cache` table.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct SkillCache {
    pub name: String,
    pub description: String,
    pub homepage: Option<String>,
    pub license: Option<String>,
    pub compatibility: Option<String>,
    pub allowed_tools: Vec<String>,
    pub dockerfile: Option<String>,
    pub requires: serde_json::Value,
    pub path: String,
    pub source: i16,
    pub content_hash: String,
    pub cached_at: DateTime<Utc>,
}
