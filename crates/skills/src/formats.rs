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

                // Handle subdirectories containing SKILL.md (e.g. skills/<name>/SKILL.md).
                if path.is_dir() {
                    let skill_md = path.join("SKILL.md");
                    if !skill_md.is_file() {
                        continue;
                    }
                    let stem = match path.file_name().and_then(|s| s.to_str()) {
                        Some(s) => s.to_string(),
                        None => continue,
                    };
                    let body = match std::fs::read_to_string(&skill_md) {
                        Ok(c) => c,
                        Err(e) => {
                            tracing::warn!(?skill_md, %e, "failed to read SKILL.md");
                            continue;
                        }
                    };

                    // Extract description: skip frontmatter and headings, take first content line.
                    let description = extract_description_from_body(&body);

                    let namespaced_name = format!("{plugin_name}:{stem}");
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

                    let source_file = skill_md
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
                        path:          path.to_path_buf(),
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
                    continue;
                }

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

    /// Scan a marketplace repo by reading `marketplace.json` and visiting each
    /// plugin's `source` directory.
    fn scan_marketplace_index(
        &self,
        repo_dir: &Path,
        marketplace_json_path: &Path,
    ) -> Vec<PluginSkillEntry> {
        let content = match std::fs::read_to_string(marketplace_json_path) {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        let index: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };
        let plugins = match index.get("plugins").and_then(|v| v.as_array()) {
            Some(p) => p,
            None => return Vec::new(),
        };

        let canonical_repo = match repo_dir.canonicalize() {
            Ok(p) => p,
            Err(_) => return Vec::new(),
        };

        let mut results = Vec::new();
        for plugin in plugins {
            let source = match plugin.get("source").and_then(|v| v.as_str()) {
                Some(s) => s,
                None => continue,
            };
            let plugin_dir = repo_dir.join(source.strip_prefix("./").unwrap_or(source));

            // Guard against path traversal (e.g. source: "../../escape").
            let canonical_plugin = match plugin_dir.canonicalize() {
                Ok(p) => p,
                Err(_) => continue,
            };
            if !canonical_plugin.starts_with(&canonical_repo) {
                tracing::warn!(
                    ?source,
                    "marketplace plugin source escapes repo directory, skipping"
                );
                continue;
            }

            if plugin_dir.join(".claude-plugin/plugin.json").is_file() {
                match self.scan_single_plugin(&plugin_dir, repo_dir) {
                    Ok(skills) => results.extend(skills),
                    Err(e) => {
                        tracing::warn!(?plugin_dir, %e, "failed to scan marketplace plugin")
                    }
                }
            }
        }
        results
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

        // Marketplace repo: read marketplace.json source fields to find plugin dirs.
        let marketplace_json_path = repo_dir.join(".claude-plugin/marketplace.json");
        let mut results = Vec::new();
        if marketplace_json_path.is_file() {
            results = self.scan_marketplace_index(repo_dir, &marketplace_json_path);
            if !results.is_empty() {
                return Ok(results);
            }
        }

        // Fallback: scan plugins/ and external_plugins/ subdirs
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

/// Extract a short description from a markdown body.
/// Skips YAML frontmatter (between `---` delimiters) and headings (`#` lines),
/// then returns the first non-empty content line, truncated to 120 chars.
fn extract_description_from_body(body: &str) -> String {
    let mut in_frontmatter = false;
    let mut frontmatter_ended = false;
    let mut lines = body.lines();

    // Check if body starts with frontmatter delimiter.
    if let Some(first) = lines.next() {
        let trimmed = first.trim();
        if trimmed == "---" {
            in_frontmatter = true;
        } else if !trimmed.is_empty() && !trimmed.starts_with('#') {
            return trimmed.chars().take(120).collect();
        }
    }

    for line in lines {
        let trimmed = line.trim();
        if in_frontmatter && !frontmatter_ended {
            if trimmed == "---" {
                frontmatter_ended = true;
                in_frontmatter = false;
            }
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        return trimmed.chars().take(120).collect();
    }
    String::new()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create a minimal plugin.json in the given directory.
    fn write_plugin_json(dir: &Path, name: &str) {
        let claude_dir = dir.join(".claude-plugin");
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::write(
            claude_dir.join("plugin.json"),
            format!(r#"{{ "name": "{name}", "description": "test plugin" }}"#),
        )
        .unwrap();
    }

    #[test]
    fn scan_marketplace_repo_with_source_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // Create marketplace.json pointing to top-level dirs (not plugins/).
        let claude_dir = root.join(".claude-plugin");
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::write(
            claude_dir.join("marketplace.json"),
            r#"{
                "name": "test-marketplace",
                "plugins": [
                    { "name": "alpha", "source": "./alpha-plugin" },
                    { "name": "beta", "source": "beta-plugin" }
                ]
            }"#,
        )
        .unwrap();

        // Create alpha-plugin with a flat skill .md file.
        let alpha = root.join("alpha-plugin");
        write_plugin_json(&alpha, "alpha");
        let skills_dir = alpha.join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::write(
            skills_dir.join("do-thing.md"),
            "# Do Thing\nDoes the thing.",
        )
        .unwrap();

        // Create beta-plugin with a subdirectory skill.
        let beta = root.join("beta-plugin");
        write_plugin_json(&beta, "beta");
        let sub_skill = beta.join("skills/my-skill");
        std::fs::create_dir_all(&sub_skill).unwrap();
        std::fs::write(sub_skill.join("SKILL.md"), "# My Skill\nA cool skill.").unwrap();

        let adapter = ClaudeCodeAdapter;
        assert!(adapter.detect(root));

        let results = adapter.scan_skills(root).unwrap();
        assert_eq!(results.len(), 2);

        let names: Vec<&str> = results.iter().map(|r| r.metadata.name.as_str()).collect();
        assert!(
            names.contains(&"alpha:do-thing"),
            "missing alpha:do-thing in {names:?}"
        );
        assert!(
            names.contains(&"beta:my-skill"),
            "missing beta:my-skill in {names:?}"
        );
    }

    #[test]
    fn scan_plugin_with_skill_subdirs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        write_plugin_json(root, "test-plugin");

        // Create skills/<name>/SKILL.md layout.
        let skill_a = root.join("skills/code-review");
        std::fs::create_dir_all(&skill_a).unwrap();
        std::fs::write(
            skill_a.join("SKILL.md"),
            "---\ntitle: Code Review\n---\n# Code Review\nReviews code changes.",
        )
        .unwrap();

        let skill_b = root.join("skills/test-gen");
        std::fs::create_dir_all(&skill_b).unwrap();
        std::fs::write(skill_b.join("SKILL.md"), "Generates unit tests.").unwrap();

        // Also a flat .md to verify both paths work together.
        std::fs::create_dir_all(root.join("agents")).unwrap();
        std::fs::write(root.join("agents/helper.md"), "Helps with stuff.").unwrap();

        let adapter = ClaudeCodeAdapter;
        let results = adapter.scan_skills(root).unwrap();

        let names: Vec<&str> = results.iter().map(|r| r.metadata.name.as_str()).collect();
        assert!(
            names.contains(&"test-plugin:code-review"),
            "missing code-review in {names:?}"
        );
        assert!(
            names.contains(&"test-plugin:test-gen"),
            "missing test-gen in {names:?}"
        );
        assert!(
            names.contains(&"test-plugin:helper"),
            "missing helper in {names:?}"
        );

        // Verify frontmatter is skipped for description extraction.
        let code_review = results
            .iter()
            .find(|r| r.metadata.name == "test-plugin:code-review")
            .unwrap();
        assert_eq!(code_review.metadata.description, "Reviews code changes.");
    }

    #[test]
    fn scan_marketplace_rejects_path_traversal() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // Create a directory outside the repo that the traversal would target.
        let escape_dir = tmp.path().join("escape-target");
        std::fs::create_dir_all(&escape_dir).unwrap();
        write_plugin_json(&escape_dir, "escaped");
        let skills_dir = escape_dir.join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::write(skills_dir.join("bad.md"), "# Bad\nShould not appear.").unwrap();

        // Create the actual repo dir nested inside tmp.
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();

        let claude_dir = repo.join(".claude-plugin");
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::write(
            claude_dir.join("marketplace.json"),
            r#"{
                "name": "evil-marketplace",
                "plugins": [
                    { "name": "escape", "source": "../escape-target" }
                ]
            }"#,
        )
        .unwrap();

        let adapter = ClaudeCodeAdapter;
        assert!(adapter.detect(&repo));

        let results = adapter.scan_skills(&repo).unwrap();
        assert!(
            results.is_empty(),
            "path traversal source should be rejected, got: {results:?}"
        );
    }

    #[test]
    fn extract_description_skips_frontmatter_and_headings() {
        let body = "---\ntitle: Foo\n---\n# Heading\n\nActual description here.";
        assert_eq!(
            extract_description_from_body(body),
            "Actual description here."
        );

        let body_no_fm = "# Heading\nSome text.";
        assert_eq!(extract_description_from_body(body_no_fm), "Some text.");

        let body_plain = "Just a line.";
        assert_eq!(extract_description_from_body(body_plain), "Just a line.");

        assert_eq!(extract_description_from_body(""), "");
    }

    #[test]
    fn hybrid_repo_detected_as_claude_code_but_scan_returns_empty() {
        // A repo with marketplace.json but no plugin.json in the source dir,
        // and native SKILL.md files — the ClaudeCode adapter should return
        // empty so the caller can fallback to SKILL.md scanning.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // Create marketplace.json pointing to root (no plugin.json there).
        let claude_dir = root.join(".claude-plugin");
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::write(
            claude_dir.join("marketplace.json"),
            r#"{
                "name": "hybrid-repo",
                "plugins": [
                    { "name": "my-skills", "source": "./" }
                ]
            }"#,
        )
        .unwrap();

        // Create native SKILL.md files.
        let skill_dir = root.join("skills/my-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: my-skill\ndescription: test\n---\n# My Skill\nDoes stuff.",
        )
        .unwrap();

        // detect_format sees marketplace.json → ClaudeCode.
        let format = detect_format(root);
        assert_eq!(format, PluginFormat::ClaudeCode);

        // But ClaudeCode scan returns empty (no plugin.json in source dir).
        let adapter = ClaudeCodeAdapter;
        let results = adapter.scan_skills(root).unwrap();
        assert!(
            results.is_empty(),
            "ClaudeCode adapter should return empty for hybrid repo, got: {results:?}"
        );

        // Verify SKILL.md files exist (caller would fallback to scan_repo_skills).
        assert!(has_skill_md_recursive(root));
    }
}
