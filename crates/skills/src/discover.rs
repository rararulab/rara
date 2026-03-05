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

//! Skill discovery from filesystem paths.
//!
//! Provides the [`SkillDiscoverer`] trait and its default implementation
//! [`FsSkillDiscoverer`], which scans project-local, personal, registry, and
//! plugin directories for `SKILL.md` files and returns parsed
//! [`SkillMetadata`].

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use snafu::ResultExt;

use crate::{
    error::{Result, TaskJoinSnafu},
    formats::PluginFormat,
    manifest::ManifestStore,
    types::{SkillMetadata, SkillSource},
};

/// Discovers skills from filesystem paths.
#[async_trait]
pub trait SkillDiscoverer: Send + Sync {
    /// Scan configured paths and return metadata for all discovered skills.
    async fn discover(&self) -> Result<Vec<SkillMetadata>>;
}

/// Default filesystem-based skill discoverer.
pub struct FsSkillDiscoverer {
    /// (path, source) pairs to scan, in priority order.
    search_paths: Vec<(PathBuf, SkillSource)>,
}

impl FsSkillDiscoverer {
    pub fn new(search_paths: Vec<(PathBuf, SkillSource)>) -> Self { Self { search_paths } }

    /// Build the default search paths for skill discovery.
    ///
    /// Scans the following locations in order:
    /// 1. Project-local: `<data_dir>/.rara/skills/`
    /// 2. Personal (rara): `<data_dir>/skills/`
    /// 3. Personal (Claude Code): `~/.claude/skills/`
    /// 4. Registry-installed: `<data_dir>/installed-skills/`
    /// 5. Plugins: `<data_dir>/installed-plugins/`
    /// 6. Bundled: `<cwd>/skills/`
    pub fn default_paths() -> Vec<(PathBuf, SkillSource)> {
        let data = rara_paths::data_dir().clone();
        let mut paths = vec![
            (data.join(".rara/skills"), SkillSource::Project),
            (data.join("skills"), SkillSource::Personal),
        ];

        // ~/.claude/skills/ — Claude Code personal skills
        if let Some(home) = dirs::home_dir() {
            paths.push((home.join(".claude/skills"), SkillSource::Personal));
        }

        paths.extend([
            (data.join("installed-skills"), SkillSource::Registry),
            (data.join("installed-plugins"), SkillSource::Plugin),
        ]);

        // Bundled skills in the working directory
        if let Ok(cwd) = std::env::current_dir() {
            let bundled = cwd.join("skills");
            if bundled.is_dir() {
                paths.push((bundled, SkillSource::Project));
            }
        }

        paths
    }
}

#[async_trait]
impl SkillDiscoverer for FsSkillDiscoverer {
    async fn discover(&self) -> Result<Vec<SkillMetadata>> {
        let search_paths = self.search_paths.clone();

        // All helpers use std::fs (blocking IO). Run on the blocking thread
        // pool to avoid stalling tokio worker threads.
        tokio::task::spawn_blocking(move || {
            let mut skills = Vec::new();

            for (base_path, source) in &search_paths {
                if !base_path.is_dir() {
                    continue;
                }

                match source {
                    SkillSource::Project | SkillSource::Personal => {
                        discover_flat(base_path, source, &mut skills);
                    }
                    SkillSource::Registry => {
                        discover_registry(base_path, &mut skills);
                    }
                    SkillSource::Plugin => {
                        discover_plugins(base_path, &mut skills);
                    }
                }
            }

            skills
        })
        .await
        .context(TaskJoinSnafu)
    }
}

/// Scan one level deep for SKILL.md dirs (project/personal sources).
fn discover_flat<P: AsRef<Path>>(
    base_path: &P,
    source: &SkillSource,
    skills: &mut Vec<SkillMetadata>,
) {
    let entries = match std::fs::read_dir(base_path) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let skill_dir = entry.path();
        if !skill_dir.is_dir() {
            continue;
        }
        let skill_md = skill_dir.join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }
        match crate::parse::parse_metadata_from_file(&skill_md, &skill_dir) {
            Ok(mut meta) => {
                meta.source = Some(source.clone());
                tracing::info!(
                    path = %skill_md.display(),
                    source = ?source,
                    name = %meta.name,
                    "loaded SKILL.md"
                );
                skills.push(meta);
            }
            Err(e) => {
                tracing::warn!(?skill_dir, %e, "failed to parse SKILL.md");
            }
        }
    }
}

/// Discover enabled plugin skills using the plugins manifest.
/// Plugin skills don't have SKILL.md -- they are normalized by format adapters.
/// This returns lightweight metadata from the manifest for prompt injection.
fn discover_plugins(install_dir: &Path, skills: &mut Vec<SkillMetadata>) {
    let manifest_path = rara_paths::data_dir().join("plugins-manifest.json");
    let store = ManifestStore::new(manifest_path);
    let manifest = match store.load() {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(%e, "failed to load plugins manifest");
            return;
        }
    };

    for repo in &manifest.repos {
        for skill_state in &repo.skills {
            if !skill_state.enabled || !skill_state.trusted {
                continue;
            }
            let skill_dir = install_dir.join(&skill_state.relative_path);
            skills.push(SkillMetadata {
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

/// Discover registry skills using the manifest for enabled filtering.
///
/// Handles both formats:
/// - `PluginFormat::Skill` -> parse `SKILL.md` from disk for full metadata
/// - Other formats -> create stub metadata with `SkillSource::Plugin`
///   (prompt_gen uses the path as-is instead of appending `/SKILL.md`)
fn discover_registry(install_dir: &Path, skills: &mut Vec<SkillMetadata>) {
    let manifest_path = match ManifestStore::default_path() {
        Ok(p) => p,
        Err(_) => return,
    };
    let store = ManifestStore::new(manifest_path);
    let manifest = match store.load() {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(%e, "failed to load skills manifest");
            return;
        }
    };

    for repo in &manifest.repos {
        for skill_state in &repo.skills {
            if !skill_state.enabled || !skill_state.trusted {
                continue;
            }
            let skill_dir = install_dir.join(&skill_state.relative_path);

            match repo.format {
                PluginFormat::Skill => {
                    let skill_md = skill_dir.join("SKILL.md");
                    if !skill_md.is_file() {
                        tracing::warn!(?skill_md, "manifest references missing SKILL.md");
                        continue;
                    }
                    match crate::parse::parse_metadata_from_file(&skill_md, &skill_dir) {
                        Ok(mut meta) => {
                            meta.source = Some(SkillSource::Registry);
                            tracing::info!(
                                path = %skill_md.display(),
                                source = "registry",
                                name = %meta.name,
                                "loaded SKILL.md"
                            );
                            skills.push(meta);
                        }
                        Err(e) => {
                            tracing::debug!(?skill_dir, %e, "skipping non-conforming SKILL.md");
                        }
                    }
                }
                _ => {
                    // Non-SKILL.md formats: stub metadata with Plugin source
                    // so prompt_gen uses the path directly (no /SKILL.md append).
                    skills.push(SkillMetadata {
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
