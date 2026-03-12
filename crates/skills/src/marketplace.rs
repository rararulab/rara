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

//! Marketplace discovery and management.
//!
//! Fetches `marketplace.json` from GitHub repos, caches plugin indexes
//! in memory, and delegates installation to [`crate::install`].

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::RwLock,
};

use serde::{Deserialize, Serialize};
use snafu::ResultExt;

use crate::error::{IoSnafu, Result, SerdeJsonSnafu};

/// A registered marketplace source, persisted to `marketplaces.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceSource {
    /// GitHub `owner/repo` identifier.
    pub repo: String,
    /// Human-friendly name (derived from repo name).
    pub name: String,
}

/// Owner metadata from marketplace.json.
#[derive(Debug, Clone, Deserialize)]
pub struct IndexOwner {
    pub name:  String,
    #[serde(default)]
    pub email: Option<String>,
}

/// Top-level marketplace index parsed from `.claude-plugin/marketplace.json`.
#[derive(Debug, Clone, Deserialize)]
pub struct MarketplaceIndex {
    pub name:        String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub owner:       Option<IndexOwner>,
    pub plugins:     Vec<MarketplacePlugin>,
}

/// A single plugin entry in the marketplace index.
#[derive(Debug, Clone, Deserialize)]
pub struct MarketplacePlugin {
    pub name:        String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub version:     Option<String>,
    #[serde(default)]
    pub category:    Option<String>,
    /// Relative path within the repo (e.g. `"./plugins/code-review"`).
    #[serde(default)]
    pub source:      Option<String>,
    /// Skill directory paths within the plugin.
    #[serde(default)]
    pub skills:      Vec<String>,
}

/// Plugin info enriched with local installation status.
#[derive(Debug, Clone, Serialize)]
pub struct PluginInfo {
    pub name:        String,
    pub description: String,
    pub version:     Option<String>,
    pub category:    Option<String>,
    pub marketplace: String,
    pub installed:   bool,
    pub enabled:     bool,
}

/// Default marketplace sources baked into the binary.
const DEFAULT_SOURCES: &[(&str, &str)] = &[
    ("anthropics/claude-plugins-official", "claude-plugins-official"),
    ("anthropics/skills", "anthropic-skills"),
];

/// Central service for marketplace operations.
pub struct MarketplaceService {
    /// Path to `marketplaces.json`.
    persist_path: PathBuf,
    /// Registered sources (persisted).
    sources: RwLock<Vec<MarketplaceSource>>,
    /// In-memory index cache, keyed by repo string.
    cache: RwLock<HashMap<String, MarketplaceIndex>>,
}

impl MarketplaceService {
    /// Create with default persistence path (`~/.config/rara/marketplaces.json`).
    pub fn new() -> Self {
        Self::new_with_path(rara_paths::config_dir().join("marketplaces.json"))
    }

    /// Create with explicit persistence path (for testing).
    pub fn new_with_path(path: PathBuf) -> Self {
        let sources = load_sources(&path);
        Self {
            persist_path: path,
            sources: RwLock::new(sources),
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// List registered marketplace sources.
    pub fn list_sources(&self) -> Vec<MarketplaceSource> {
        self.sources.read().unwrap().clone()
    }

    /// Register a new marketplace source by `owner/repo`.
    pub fn add_source(&self, repo: &str) -> Result<()> {
        let mut sources = self.sources.write().unwrap();
        if sources.iter().any(|s| s.repo == repo) {
            return Ok(()); // idempotent
        }
        let name = repo.split('/').last().unwrap_or(repo).to_string();
        sources.push(MarketplaceSource {
            repo: repo.to_string(),
            name,
        });
        save_sources(&self.persist_path, &sources)
    }

    /// Remove a marketplace source.
    pub fn remove_source(&self, repo: &str) -> Result<()> {
        let mut sources = self.sources.write().unwrap();
        sources.retain(|s| s.repo != repo);
        self.cache.write().unwrap().remove(repo);
        save_sources(&self.persist_path, &sources)
    }

    /// Fetch the marketplace index from GitHub API (single file, no full clone).
    ///
    /// GET /repos/{owner}/{repo}/contents/.claude-plugin/marketplace.json
    /// → base64 decode → parse as MarketplaceIndex → cache.
    pub async fn fetch_index(&self, repo: &str) -> Result<MarketplaceIndex> {
        // Return cached if available.
        if let Some(idx) = self.cache.read().unwrap().get(repo) {
            return Ok(idx.clone());
        }

        let url = format!(
            "https://api.github.com/repos/{repo}/contents/.claude-plugin/marketplace.json"
        );
        let client = reqwest::Client::new();
        let resp: serde_json::Value = client
            .get(&url)
            .header("User-Agent", "rara-marketplace")
            .header("Accept", "application/vnd.github.v3+json")
            .send()
            .await
            .context(crate::error::RequestSnafu)?
            .json()
            .await
            .context(crate::error::RequestSnafu)?;

        let content_b64 = resp
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| crate::error::SkillError::InvalidInput {
                message: format!("no 'content' field in GitHub API response for {repo}"),
            })?;

        // GitHub returns base64 with newlines.
        use base64::Engine;
        let cleaned: String = content_b64.chars().filter(|c| !c.is_whitespace()).collect();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&cleaned)
            .map_err(|e| crate::error::SkillError::InvalidInput {
                message: format!("base64 decode failed: {e}"),
            })?;

        let index: MarketplaceIndex =
            serde_json::from_slice(&bytes).context(crate::error::SerdeJsonSnafu)?;

        self.cache
            .write()
            .unwrap()
            .insert(repo.to_string(), index.clone());
        Ok(index)
    }

    /// Clear cached indexes. Next `fetch_index` will re-fetch from GitHub.
    pub fn clear_cache(&self) {
        self.cache.write().unwrap().clear();
    }

    /// Clear cache for a specific marketplace.
    pub fn clear_cache_for(&self, repo: &str) {
        self.cache.write().unwrap().remove(repo);
    }
}

/// Load sources from JSON file, seeding defaults if missing.
fn load_sources(path: &PathBuf) -> Vec<MarketplaceSource> {
    let mut sources: Vec<MarketplaceSource> = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    // Ensure defaults are present.
    for &(repo, name) in DEFAULT_SOURCES {
        if !sources.iter().any(|s| s.repo == repo) {
            sources.push(MarketplaceSource {
                repo: repo.to_string(),
                name: name.to_string(),
            });
        }
    }
    // Best-effort persist if defaults were added.
    let _ = save_sources(path, &sources);
    sources
}

/// Persist sources to JSON.
fn save_sources(path: &PathBuf, sources: &[MarketplaceSource]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context(IoSnafu)?;
    }
    let json = serde_json::to_string_pretty(sources).context(SerdeJsonSnafu)?;
    std::fs::write(path, json).context(IoSnafu)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_sources_include_official_repos() {
        let svc = MarketplaceService::new_with_path(
            tempfile::NamedTempFile::new().unwrap().path().to_path_buf(),
        );
        let sources = svc.list_sources();
        assert_eq!(sources.len(), 2);
        assert!(sources.iter().any(|s| s.repo == "anthropics/claude-plugins-official"));
        assert!(sources.iter().any(|s| s.repo == "anthropics/skills"));
    }

    #[test]
    fn add_and_remove_source() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let svc = MarketplaceService::new_with_path(tmp.path().to_path_buf());

        svc.add_source("my-org/my-marketplace").unwrap();
        let sources = svc.list_sources();
        assert_eq!(sources.len(), 3);

        svc.remove_source("my-org/my-marketplace").unwrap();
        let sources = svc.list_sources();
        assert_eq!(sources.len(), 2);
    }
}
