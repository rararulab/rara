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

use std::{collections::HashMap, path::PathBuf, sync::RwLock};

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

/// Result of a plugin installation.
#[derive(Debug, Clone, Serialize)]
pub struct PluginInstallResult {
    pub plugin:       String,
    pub skills_count: usize,
    pub skills:       Vec<String>,
}

/// Default marketplace sources baked into the binary.
const DEFAULT_SOURCES: &[(&str, &str)] = &[
    (
        "anthropics/claude-plugins-official",
        "claude-plugins-official",
    ),
    ("anthropics/skills", "anthropic-skills"),
];

/// Central service for marketplace operations.
pub struct MarketplaceService {
    /// Path to `marketplaces.json`.
    persist_path: PathBuf,
    /// Registered sources (persisted).
    sources:      RwLock<Vec<MarketplaceSource>>,
    /// In-memory index cache, keyed by repo string.
    cache:        RwLock<HashMap<String, MarketplaceIndex>>,
    /// Optional live skill registry for immediate updates on
    /// install/enable/disable.
    registry:     Option<crate::registry::InMemoryRegistry>,
}

impl MarketplaceService {
    /// Create with default persistence path
    /// (`~/.config/rara/marketplaces.json`).
    pub fn new() -> Self { Self::new_with_path(rara_paths::config_dir().join("marketplaces.json")) }

    /// Create with explicit persistence path (for testing).
    pub fn new_with_path(path: PathBuf) -> Self {
        let sources = load_sources(&path);
        Self {
            persist_path: path,
            sources:      RwLock::new(sources),
            cache:        RwLock::new(HashMap::new()),
            registry:     None,
        }
    }

    /// Attach a skill registry for live updates on install/enable/disable.
    #[must_use]
    pub fn with_registry(mut self, registry: crate::registry::InMemoryRegistry) -> Self {
        self.registry = Some(registry);
        self
    }

    /// List registered marketplace sources.
    pub fn list_sources(&self) -> Vec<MarketplaceSource> { self.sources.read().unwrap().clone() }

    /// Register a new marketplace source by `owner/repo`.
    pub fn add_source(&self, repo: &str) -> Result<()> {
        let mut sources = self.sources.write().unwrap();
        if sources.iter().any(|s| s.repo == repo) {
            return Ok(()); // idempotent
        }
        let name = repo.split('/').next_back().unwrap_or(repo).to_string();
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

    /// Fetch the marketplace index from GitHub API (single file, no full
    /// clone).
    ///
    /// GET /repos/{owner}/{repo}/contents/.claude-plugin/marketplace.json
    /// → base64 decode → parse as MarketplaceIndex → cache.
    pub async fn fetch_index(&self, repo: &str) -> Result<MarketplaceIndex> {
        // Return cached if available.
        if let Some(idx) = self.cache.read().unwrap().get(repo) {
            return Ok(idx.clone());
        }

        let gh = crate::github::GitHubClient::new();

        // Try marketplace.json first; fall back to plugin.json ONLY when the
        // file is genuinely absent (404).  Non-retriable errors (403 bad token,
        // 429 rate limit after retries exhausted, 5xx) must propagate
        // immediately so they are not masked by a confusing "has neither
        // marketplace.json nor plugin.json" message.
        let content_b64 = match self
            .fetch_github_content_b64(&gh, repo, "marketplace.json")
            .await
        {
            Ok(b64) => b64,
            Err(crate::error::SkillError::HttpStatus { status: 404, .. }) => {
                // Fallback: try .claude-plugin/plugin.json for single-plugin repos.
                let fallback_b64 = self
                    .fetch_github_content_b64(&gh, repo, "plugin.json")
                    .await
                    .map_err(|e| {
                        tracing::debug!(%e, repo, "plugin.json fallback also failed");
                        crate::error::SkillError::InvalidInput {
                            message: format!(
                                "repo '{repo}' has neither marketplace.json nor plugin.json"
                            ),
                        }
                    })?;

                use base64::Engine;
                let cleaned: String = fallback_b64
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect();
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(&cleaned)
                    .map_err(|e| crate::error::SkillError::InvalidInput {
                        message: format!("base64 decode failed: {e}"),
                    })?;
                let json_str = String::from_utf8(bytes).map_err(|e| {
                    crate::error::SkillError::InvalidInput {
                        message: format!("plugin.json is not valid UTF-8: {e}"),
                    }
                })?;

                let index = synthetic_index_from_plugin_json(repo, &json_str)?;
                self.cache
                    .write()
                    .unwrap()
                    .insert(repo.to_string(), index.clone());
                return Ok(index);
            }
            // Non-404 errors (rate limit, auth failure, server error) —
            // propagate immediately.
            Err(e) => return Err(e),
        };

        // GitHub returns base64 with newlines.
        use base64::Engine;
        let cleaned: String = content_b64
            .chars()
            .filter(|c: &char| !c.is_whitespace())
            .collect();
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
    pub fn clear_cache(&self) { self.cache.write().unwrap().clear(); }

    /// Clear cache for a specific marketplace.
    pub fn clear_cache_for(&self, repo: &str) { self.cache.write().unwrap().remove(repo); }

    /// Browse all plugins across all (or one) marketplace.
    pub async fn browse(&self, marketplace: Option<&str>) -> Result<Vec<PluginInfo>> {
        let sources = self.list_sources();
        let targets: Vec<&MarketplaceSource> = match marketplace {
            Some(name) => sources
                .iter()
                .filter(|s| s.name == name || s.repo == name)
                .collect(),
            None => sources.iter().collect(),
        };

        let manifest = self.load_install_manifest();
        let mut results = Vec::new();

        for src in targets {
            let index = match self.fetch_index(&src.repo).await {
                Ok(idx) => idx,
                Err(e) => {
                    tracing::warn!(repo = %src.repo, %e, "failed to fetch marketplace index");
                    continue;
                }
            };
            for plugin in &index.plugins {
                let (installed, enabled) =
                    self.plugin_local_status(&manifest, &src.repo, &plugin.name);
                results.push(PluginInfo {
                    name: plugin.name.clone(),
                    description: plugin.description.clone().unwrap_or_default(),
                    version: plugin.version.clone(),
                    category: plugin.category.clone(),
                    marketplace: src.name.clone(),
                    installed,
                    enabled,
                });
            }
        }
        Ok(results)
    }

    /// Search plugins by matching query against name, description, category.
    pub async fn search(&self, query: &str) -> Result<Vec<PluginInfo>> {
        let all = self.browse(None).await?;
        let q = query.to_lowercase();
        Ok(all
            .into_iter()
            .filter(|p| {
                p.name.to_lowercase().contains(&q)
                    || p.description.to_lowercase().contains(&q)
                    || p.category
                        .as_ref()
                        .is_some_and(|c| c.to_lowercase().contains(&q))
            })
            .collect())
    }

    /// Install a plugin from a marketplace.
    pub async fn install_plugin(
        &self,
        plugin_name: &str,
        marketplace: Option<&str>,
    ) -> Result<PluginInstallResult> {
        // Find which marketplace has this plugin.
        let all = self.browse(marketplace).await?;
        let info = all.iter().find(|p| p.name == plugin_name).ok_or_else(|| {
            crate::error::SkillError::NotFound {
                name: format!("plugin '{plugin_name}' not found in any marketplace"),
            }
        })?;

        let source_repo = self
            .list_sources()
            .iter()
            .find(|s| s.name == info.marketplace)
            .map(|s| s.repo.clone())
            .ok_or_else(|| crate::error::SkillError::NotFound {
                name: format!("marketplace '{}' not found", info.marketplace),
            })?;

        let install_dir = crate::install::default_install_dir()?;

        let manifest_path = crate::manifest::ManifestStore::default_path()?;
        let store = crate::manifest::ManifestStore::new(manifest_path);

        // Check if repo is already installed; if not, download it first.
        // Note: this read is intentionally outside the lock — holding the lock
        // across the HTTP download would block all concurrent manifest access.
        // TOCTOU is acceptable: install_skill() has its own conflict handling.
        let needs_install = store.load()?.find_repo(&source_repo).is_none();
        if needs_install {
            crate::install::install_skill(&source_repo, &install_dir).await?;
        }

        // Enable matching skills under exclusive lock.
        let (enabled_skills, manifest_snapshot) = store.with_lock(|manifest| {
            let plugin = plugin_name.to_string();
            let mut enabled = Vec::new();
            if let Some(repo_entry) = manifest.find_repo_mut(&source_repo) {
                for skill in &mut repo_entry.skills {
                    if skill.name.starts_with(&format!("{plugin}:")) || skill.name == plugin {
                        skill.enabled = true;
                        skill.trusted = true;
                        enabled.push(skill.name.clone());
                    }
                }
            }
            Ok((enabled, manifest.clone()))
        })?;

        // Update in-memory registry so the agent prompt reflects new skills
        // immediately.
        self.sync_repo_to_registry(&manifest_snapshot, &source_repo);

        Ok(PluginInstallResult {
            plugin:       plugin_name.to_string(),
            skills_count: enabled_skills.len(),
            skills:       enabled_skills,
        })
    }

    /// Install all plugins from a GitHub repo directly by `owner/repo`.
    ///
    /// Adds the repo as a marketplace source, downloads it, scans for skills,
    /// and enables all discovered skills.
    pub async fn install_repo(&self, repo: &str) -> Result<PluginInstallResult> {
        // Normalize source to "owner/repo" format so manifest lookups match
        // the normalized key that install_skill() stores internally.
        let (owner, repo_name_parsed) = crate::install::parse_source(repo)?;
        let normalized = format!("{owner}/{repo_name_parsed}");

        // Install first; only register the source after a successful download
        // so a failed install does not leave a stale source entry.
        let install_dir = crate::install::default_install_dir()?;
        crate::install::install_skill(&normalized, &install_dir).await?;

        self.add_source(&normalized)?;

        // Enable all skills from this repo under exclusive lock.
        let manifest_path = crate::manifest::ManifestStore::default_path()?;
        let store = crate::manifest::ManifestStore::new(manifest_path);

        let (enabled_skills, manifest_snapshot) = store.with_lock(|manifest| {
            let mut enabled = Vec::new();
            if let Some(repo_entry) = manifest.find_repo_mut(&normalized) {
                for skill in &mut repo_entry.skills {
                    skill.enabled = true;
                    skill.trusted = true;
                    enabled.push(skill.name.clone());
                }
            }
            Ok((enabled, manifest.clone()))
        })?;

        // Update in-memory registry so the agent prompt reflects new skills
        // immediately.
        self.sync_repo_to_registry(&manifest_snapshot, &normalized);

        let display_name = repo.split('/').next_back().unwrap_or(repo);
        Ok(PluginInstallResult {
            plugin:       display_name.to_string(),
            skills_count: enabled_skills.len(),
            skills:       enabled_skills,
        })
    }

    /// Enable a previously installed plugin.
    pub fn enable_plugin(&self, plugin_name: &str) -> Result<()> {
        self.set_plugin_state(plugin_name, true)
    }

    /// Disable a plugin without uninstalling.
    pub fn disable_plugin(&self, plugin_name: &str) -> Result<()> {
        self.set_plugin_state(plugin_name, false)
    }

    fn set_plugin_state(&self, plugin_name: &str, enabled: bool) -> Result<()> {
        let manifest_path = crate::manifest::ManifestStore::default_path()?;
        let store = crate::manifest::ManifestStore::new(manifest_path);

        let (affected_names, manifest_snapshot) = store.with_lock(|manifest| {
            let plugin = plugin_name.to_string();
            let mut found = false;
            let mut affected = Vec::new();
            for repo in &mut manifest.repos {
                for skill in &mut repo.skills {
                    if skill.name.starts_with(&format!("{plugin}:")) || skill.name == plugin {
                        skill.enabled = enabled;
                        if enabled {
                            skill.trusted = true;
                        }
                        affected.push(skill.name.clone());
                        found = true;
                    }
                }
            }
            if !found {
                return Err(crate::error::SkillError::NotFound {
                    name: format!("plugin '{plugin}' is not installed"),
                });
            }
            Ok((affected, manifest.clone()))
        })?;

        // Update in-memory registry for immediate effect.
        if let Some(ref registry) = self.registry {
            if enabled {
                // Re-discover and insert enabled skills.
                for repo in &manifest_snapshot.repos {
                    self.sync_repo_to_registry(&manifest_snapshot, &repo.source);
                }
            } else {
                // Remove disabled skills from the registry.
                for name in &affected_names {
                    registry.remove(name);
                }
            }
        }

        Ok(())
    }
}

// Private helpers.
impl MarketplaceService {
    /// Fetch the base64 `content` field from a GitHub Contents API response.
    ///
    /// Used by [`fetch_index`](Self::fetch_index) to retrieve JSON files from
    /// `.claude-plugin/` without cloning the entire repo.
    async fn fetch_github_content_b64(
        &self,
        gh: &crate::github::GitHubClient,
        repo: &str,
        filename: &str,
    ) -> Result<String> {
        let url = format!("https://api.github.com/repos/{repo}/contents/.claude-plugin/{filename}");
        let resp = gh.get(&url, &format!("{filename} fetch")).await?;
        let body: serde_json::Value = resp.json().await.context(crate::error::RequestSnafu)?;
        body.get("content")
            .and_then(|v| v.as_str())
            .map(ToString::to_string)
            .ok_or_else(|| crate::error::SkillError::InvalidInput {
                message: format!("{filename} for '{repo}' has no 'content' field"),
            })
    }

    /// Re-discover enabled skills for a repo and insert them into the
    /// in-memory registry.
    fn sync_repo_to_registry(&self, manifest: &crate::types::SkillsManifest, source_repo: &str) {
        use crate::{
            formats::PluginFormat,
            types::{SkillMetadata, SkillSource},
        };

        let registry = match self.registry {
            Some(ref r) => r,
            None => return,
        };
        let install_dir = match crate::install::default_install_dir() {
            Ok(d) => d,
            Err(_) => return,
        };
        if let Some(repo_entry) = manifest.find_repo(source_repo) {
            for skill_state in &repo_entry.skills {
                if !skill_state.enabled {
                    continue;
                }
                let skill_dir = install_dir.join(&skill_state.relative_path);

                match repo_entry.format {
                    PluginFormat::Skill => {
                        // Native SKILL.md format: parse full metadata.
                        let skill_md = skill_dir.join("SKILL.md");
                        if skill_md.is_file() {
                            if let Ok(content) = std::fs::read_to_string(&skill_md) {
                                if let Ok(mut meta) =
                                    crate::parse::parse_metadata(&content, &skill_dir)
                                {
                                    meta.source = Some(SkillSource::Registry);
                                    registry.insert(meta);
                                }
                            }
                        }
                    }
                    _ => {
                        // Non-SKILL.md formats (ClaudeCode, Codex, Generic):
                        // stub metadata with Plugin source, matching discover_registry.
                        registry.insert(SkillMetadata {
                            name:          skill_state.name.clone(),
                            description:   String::new(),
                            homepage:      None,
                            license:       None,
                            compatibility: None,
                            allowed_tools: Vec::new(),
                            requires:      Default::default(),
                            path:          skill_dir,
                            source:        Some(SkillSource::Plugin),
                            dockerfile:    None,
                        });
                    }
                }
            }
        }
    }

    fn load_install_manifest(&self) -> crate::types::SkillsManifest {
        crate::manifest::ManifestStore::default_path()
            .ok()
            .map(crate::manifest::ManifestStore::new)
            .and_then(|s| s.load().ok())
            .unwrap_or_default()
    }

    fn plugin_local_status(
        &self,
        manifest: &crate::types::SkillsManifest,
        _repo: &str,
        plugin_name: &str,
    ) -> (bool, bool) {
        for repo in &manifest.repos {
            for skill in &repo.skills {
                if skill.name.starts_with(&format!("{plugin_name}:")) || skill.name == plugin_name {
                    return (true, skill.enabled);
                }
            }
        }
        (false, false)
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

/// Build a synthetic [`MarketplaceIndex`] from a single-plugin repo's
/// `.claude-plugin/plugin.json` content. Used as a fallback when the repo has
/// no `marketplace.json`.
fn synthetic_index_from_plugin_json(repo: &str, json_str: &str) -> Result<MarketplaceIndex> {
    #[derive(Deserialize)]
    struct PluginJson {
        name:        String,
        #[serde(default)]
        description: Option<String>,
    }
    let pj: PluginJson = serde_json::from_str(json_str).context(SerdeJsonSnafu)?;
    let repo_name = repo.split('/').next_back().unwrap_or(repo);
    Ok(MarketplaceIndex {
        name:        repo_name.to_string(),
        description: pj.description.clone(),
        owner:       None,
        plugins:     vec![MarketplacePlugin {
            name:        pj.name,
            description: pj.description,
            version:     None,
            category:    None,
            source:      Some(".".to_string()),
            skills:      Vec::new(),
        }],
    })
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
        assert!(
            sources
                .iter()
                .any(|s| s.repo == "anthropics/claude-plugins-official")
        );
        assert!(sources.iter().any(|s| s.repo == "anthropics/skills"));
    }

    #[test]
    fn marketplace_index_deserializes_from_official_format() {
        let json = r#"{
            "name": "test-marketplace",
            "description": "Test",
            "owner": { "name": "Test", "email": "test@test.com" },
            "plugins": [
                {
                    "name": "code-review",
                    "description": "Code review tools",
                    "version": "1.0.0",
                    "category": "development",
                    "source": "./plugins/code-review",
                    "strict": false,
                    "skills": ["./skills/reviewer"]
                }
            ]
        }"#;
        let index: MarketplaceIndex = serde_json::from_str(json).unwrap();
        assert_eq!(index.plugins.len(), 1);
        assert_eq!(index.plugins[0].name, "code-review");
        assert_eq!(index.plugins[0].category.as_deref(), Some("development"));
    }

    #[test]
    fn synthetic_index_from_single_plugin_json() {
        let json = r#"{ "name": "my-cool-plugin", "description": "Does cool things" }"#;
        let index = super::synthetic_index_from_plugin_json("acme/cool-repo", json).unwrap();
        assert_eq!(index.name, "cool-repo");
        assert_eq!(index.description.as_deref(), Some("Does cool things"));
        assert_eq!(index.plugins.len(), 1);
        assert_eq!(index.plugins[0].name, "my-cool-plugin");
        assert_eq!(index.plugins[0].source.as_deref(), Some("."));
    }

    #[test]
    fn synthetic_index_without_description() {
        let json = r#"{ "name": "minimal" }"#;
        let index = super::synthetic_index_from_plugin_json("org/repo", json).unwrap();
        assert_eq!(index.plugins[0].name, "minimal");
        assert!(index.plugins[0].description.is_none());
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
