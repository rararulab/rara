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

//! ClawHub marketplace client - search and install skills from clawhub.ai.
//!
//! ClawHub is a public skill registry with vector search, versioning, and
//! moderation. This client wraps the v1 REST API.
//!
//! API reference: <https://clawhub.ai/api/v1/>
//! - Search:   `GET /api/v1/search?q=...&limit=20`
//! - Browse:   `GET /api/v1/skills?limit=20&sort=trending`
//! - Detail:   `GET /api/v1/skills/{slug}`
//! - Download: `GET /api/v1/download?slug=...`

use serde::{Deserialize, Serialize};

// -- Search: GET /api/v1/search?q=...&limit=N --------------------------------

/// A skill entry from the search endpoint.
///
/// Search results use `results` (not `items`) and are flatter than browse.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClawhubSearchEntry {
    pub slug:         String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub summary:      String,
    #[serde(default)]
    pub version:      Option<String>,
    #[serde(default)]
    pub score:        f64,
    /// Unix ms timestamp.
    #[serde(default)]
    pub updated_at:   Option<i64>,
}

/// Response from `GET /api/v1/search`.
#[derive(Debug, Clone, Deserialize)]
pub struct ClawhubSearchResponse {
    pub results: Vec<ClawhubSearchEntry>,
}

// -- Browse: GET /api/v1/skills?limit=N&sort=... -----------------------------

/// Stats nested inside browse entries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClawhubStats {
    #[serde(default)]
    pub downloads:         u64,
    #[serde(default)]
    pub installs_all_time: u64,
    #[serde(default)]
    pub installs_current:  u64,
    #[serde(default)]
    pub stars:             u64,
}

/// Version info nested inside browse entries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClawhubVersionInfo {
    #[serde(default)]
    pub version:    String,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub changelog:  String,
}

/// A skill entry from the browse endpoint (`GET /api/v1/skills`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClawhubBrowseEntry {
    pub slug:           String,
    #[serde(default)]
    pub display_name:   String,
    #[serde(default)]
    pub summary:        String,
    #[serde(default)]
    pub tags:           std::collections::HashMap<String, String>,
    #[serde(default)]
    pub stats:          ClawhubStats,
    #[serde(default)]
    pub created_at:     i64,
    #[serde(default)]
    pub updated_at:     i64,
    #[serde(default)]
    pub latest_version: Option<ClawhubVersionInfo>,
}

/// Paginated response from the browse endpoint.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClawhubBrowseResponse {
    pub items:       Vec<ClawhubBrowseEntry>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

// -- Detail: GET /api/v1/skills/{slug} ---------------------------------------

/// Owner info from the skill detail endpoint.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClawhubOwner {
    #[serde(default)]
    pub handle:       Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
}

/// The `skill` object nested inside the detail response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClawhubSkillInfo {
    pub slug:         String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub summary:      String,
    #[serde(default)]
    pub stats:        ClawhubStats,
    #[serde(default)]
    pub updated_at:   i64,
}

/// Full detail response from `GET /api/v1/skills/{slug}`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClawhubSkillDetail {
    pub skill:          ClawhubSkillInfo,
    #[serde(default)]
    pub latest_version: Option<ClawhubVersionInfo>,
    #[serde(default)]
    pub owner:          Option<ClawhubOwner>,
}

// -- Sort enum ----------------------------------------------------------------

/// Sort order for browsing skills on ClawHub.
#[derive(Debug, Clone, Copy)]
pub enum ClawhubSort {
    Trending,
    Updated,
    Downloads,
    Stars,
}

impl ClawhubSort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Trending => "trending",
            Self::Updated => "updated",
            Self::Downloads => "downloads",
            Self::Stars => "stars",
        }
    }
}
