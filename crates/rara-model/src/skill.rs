// Copyright 2025 Crrow
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

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

/// Cached skill metadata row from `skill_cache` table.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct SkillCache {
    pub name:          String,
    pub description:   String,
    pub homepage:      Option<String>,
    pub license:       Option<String>,
    pub compatibility: Option<String>,
    pub allowed_tools: Vec<String>,
    pub dockerfile:    Option<String>,
    pub requires:      serde_json::Value,
    pub path:          String,
    pub source:        i16,
    pub content_hash:  String,
    pub cached_at:     DateTime<Utc>,
}
