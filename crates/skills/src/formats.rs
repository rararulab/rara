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

//! Repo format detection and adapters.
//!
//! Different AI coding tools use different layouts for their plugin/skill
//! repos. This module detects the format and normalizes repo contents into
//! `SkillMetadata` + body pairs that feed into the skills system.

use std::path::Path;

use serde::{Deserialize, Serialize};
use snafu::ResultExt;

use crate::{
    error::{IoSnafu, Result, SerdeJsonSnafu},
    types::{SkillMetadata, SkillRequirements, SkillSource},
};

// ── Plugin format enum ──────────────────────────────────────────────────────

/// Detected format of a plugin/skill repository.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    strum::EnumString,
    strum::AsRefStr,
    strum::Display,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum PluginFormat {
    /// Native `SKILL.md` format (single or multi-skill repo).
    #[default]
    Skill,
    /// Claude Code plugin: `.claude-plugin/plugin.json` + `agents/`,
    /// `commands/`, `skills/` dirs.
    ClaudeCode,
    /// Codex plugin: `codex-plugin.json` or `.codex/plugin.json` (future).
    Codex,
    /// Fallback: `.md` files treated as generic skill prompts.
    Generic,
}

// ── Plugin skill entry ──────────────────────────────────────────────────────

/// A single skill entry scanned from a non-SKILL.md repo, with extra metadata
/// beyond what `SkillMetadata` carries.
#[derive(Debug, Clone, Serialize)]
pub struct PluginSkillEntry {
    pub metadata:     SkillMetadata,
    pub body:         String,
    /// Human-friendly display name (e.g. "Code Reviewer" for `code-reviewer`).
    pub display_name: Option<String>,
    /// Plugin author (from plugin.json).
    pub author:       Option<String>,
    /// Relative path of the source `.md` file within the repo (e.g.
    /// `agents/code-reviewer.md`).
    pub source_file:  Option<String>,
}

// ── Format adapter trait ────────────────────────────────────────────────────

/// A format adapter normalizes a non-SKILL.md repo into skill entries.
pub trait FormatAdapter: Send + Sync {
    /// Check whether the given repo directory matches this format.
    fn detect(&self, repo_dir: &Path) -> bool;

    /// Scan the repo and return enriched entries for each skill found.
    fn scan_skills(&self, repo_dir: &Path) -> Result<Vec<PluginSkillEntry>>;
}

// ── Claude Code adapter ─────────────────────────────────────────────────────

/// Claude Code plugin metadata from `.claude-plugin/plugin.json`.
#[derive(Debug, Deserialize)]
struct ClaudePluginJson {
    name:        String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    author:      Option<PluginAuthor>,
}

/// Author field can be a string or an object with `name` (and optionally
/// `email`).
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PluginAuthor {
    Simple(String),
    Object { name: String },
}

impl PluginAuthor {
    fn name(&self) -> &str {
        match self {
            Self::Simple(s) => s,
            Self::Object { name } => name,
        }
    }
}

/// Adapter for Claude Code plugin repos.
pub struct ClaudeCodeAdapter;

impl ClaudeCodeAdapter {
    /// Scan a single plugin directory (one that has
    /// `.claude-plugin/plugin.json`). `repo_root` is the top-level repo
    /// directory; `source_file` paths are computed relative to it so that
    /// GitHub URLs work for marketplace repos.
    fn scan_single_plugin(
        &self,
        plugin_dir: &Path,
        repo_root: &Path,
    ) -> Result<Vec<PluginSkillEntry>> {
        let plugin_json_path = plugin_dir.join(".claude-plugin/plugin.json");
        let plugin_json: ClaudePluginJson =
            serde_json::from_str(&std::fs::read_to_string(&plugin_json_path).context(IoSnafu)?)
                .context(SerdeJsonSnafu)?;

        let plugin_name = &plugin_json.name;
        let author = plugin_json.author.as_ref().map(|a| a.name().to_string());
        let mut results = Vec::new();

        // Scan agents/, commands/, skills/ directories for .md files.
        for subdir in &["agents", "commands", "skills"] {
            let dir = plugin_dir.join(subdir);
            if !dir.is_dir() {
                continue;
            }
            let entries = match std::fs::read_dir(&dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let ext = path.extension().and_then(|e: &std::ffi::OsStr| e.to_str());
                if ext != Some("md") {
                    continue;
                }
                let stem = match path.file_stem().and_then(|s: &std::ffi::OsStr| s.to_str()) {
                    Some(s) => s.to_string(),
                    None => continue,
                };

                let body = match std::fs::read_to_string(&path) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!(?path, %e, "failed to read plugin skill file");
                        continue;
                    }
                };

                // Extract description from first non-empty line of body.
                let description = body
                    .lines()
                    .find(|l| {
                        let trimmed = l.trim();
                        !trimmed.is_empty() && !trimmed.starts_with('#')
                    })
                    .unwrap_or("")
                    .trim()
                    .chars()
                    .take(120)
                    .collect::<String>();

                let namespaced_name = format!("{plugin_name}:{stem}");

                // Build display name from stem: "code-reviewer" -> "Code Reviewer"
                let display_name = stem
                    .split('-')
                    .map(|w| {
                        let mut c = w.chars();
                        match c.next() {
                            Some(first) => first.to_uppercase().to_string() + c.as_str(),
                            None => String::new(),
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ");

                // Relative path within repo root (e.g.
                // "plugins/pr-review-toolkit/agents/code-reviewer.md")
                let source_file = path
                    .strip_prefix(repo_root)
                    .ok()
                    .map(|p| p.to_string_lossy().to_string());

                let meta = SkillMetadata {
                    name:          namespaced_name,
                    description:   if description.is_empty() {
                        plugin_json.description.clone().unwrap_or_default()
                    } else {
                        description
                    },
                    homepage:      author.as_ref().map(|a| format!("https://github.com/{a}")),
                    license:       None,
                    compatibility: None,
                    allowed_tools: Vec::new(),
                    requires:      SkillRequirements::default(),
                    path:          path.parent().unwrap_or(plugin_dir).to_path_buf(),
                    source:        Some(SkillSource::Plugin),
                    dockerfile:    None,
                };

                results.push(PluginSkillEntry {
                    metadata: meta,
                    body,
                    display_name: Some(display_name),
                    author: author.clone(),
                    source_file,
                });
            }
        }

        Ok(results)
    }
}

impl FormatAdapter for ClaudeCodeAdapter {
    fn detect(&self, repo_dir: &Path) -> bool {
        // Single plugin: .claude-plugin/plugin.json at root
        // Marketplace repo: .claude-plugin/marketplace.json at root
        repo_dir.join(".claude-plugin/plugin.json").is_file()
            || repo_dir.join(".claude-plugin/marketplace.json").is_file()
    }

    fn scan_skills(&self, repo_dir: &Path) -> Result<Vec<PluginSkillEntry>> {
        // Single plugin case
        if repo_dir.join(".claude-plugin/plugin.json").is_file() {
            return self.scan_single_plugin(repo_dir, repo_dir);
        }

        // Marketplace repo: scan plugins/ and external_plugins/ subdirs
        let mut results = Vec::new();
        for container in &["plugins", "external_plugins"] {
            let container_dir = repo_dir.join(container);
            if !container_dir.is_dir() {
                continue;
            }
            let entries = match std::fs::read_dir(&container_dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                if !path.join(".claude-plugin/plugin.json").is_file() {
                    continue;
                }
                match self.scan_single_plugin(&path, repo_dir) {
                    Ok(skills) => results.extend(skills),
                    Err(e) => {
                        tracing::warn!(?path, %e, "failed to scan sub-plugin");
                    }
                }
            }
        }

        Ok(results)
    }
}

// ── Format detection ────────────────────────────────────────────────────────

/// All known format adapters, in detection priority order.
fn adapters() -> Vec<(PluginFormat, Box<dyn FormatAdapter>)> {
    vec![(PluginFormat::ClaudeCode, Box::new(ClaudeCodeAdapter))]
}

/// Detect the format of a repository.
pub fn detect_format(repo_dir: &Path) -> PluginFormat {
    for (format, adapter) in adapters() {
        if adapter.detect(repo_dir) {
            return format;
        }
    }

    // Check for native SKILL.md.
    if repo_dir.join("SKILL.md").is_file() || has_skill_md_recursive(repo_dir) {
        return PluginFormat::Skill;
    }

    PluginFormat::Generic
}

/// Scan a repo using the detected format adapter.
/// Returns `None` for `Skill` format (caller should use existing SKILL.md
/// scanning).
pub fn scan_with_adapter(
    repo_dir: &Path,
    format: PluginFormat,
) -> Option<Result<Vec<PluginSkillEntry>>> {
    match format {
        PluginFormat::Skill => None, // handled by existing scan_repo_skills
        PluginFormat::ClaudeCode => Some(ClaudeCodeAdapter.scan_skills(repo_dir)),
        PluginFormat::Codex => None, // not yet implemented
        PluginFormat::Generic => None,
    }
}

/// Check if there's at least one SKILL.md in subdirectories.
fn has_skill_md_recursive(dir: &Path) -> bool {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return false,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.join("SKILL.md").is_file() {
                return true;
            }
            if has_skill_md_recursive(&path) {
                return true;
            }
        }
    }
    false
}
