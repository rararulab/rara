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

//! Skill installation from GitHub repositories.
//!
//! Downloads a repo as an HTTP tarball, extracts it, auto-detects the repo
//! format (`SKILL.md`, Claude Code `.claude-plugin/`, etc.), scans for skills,
//! and records the installation in a persistent [`ManifestStore`].

use std::path::{Component, Path, PathBuf};

use snafu::ResultExt;

use crate::{
    error::{
        ArchiveSnafu, InstallSnafu, InvalidInputSnafu, IoSnafu, NotFoundSnafu, RequestSnafu,
        Result, TaskJoinSnafu,
    },
    formats::{PluginFormat, detect_format, scan_with_adapter},
    manifest::ManifestStore,
    parse,
    types::{RepoEntry, SkillMetadata, SkillState},
};

/// Install a skill repo from GitHub into the target directory.
///
/// Downloads the repo to `install_dir/<owner>-<repo>/`, auto-detects its format
/// (SKILL.md, Claude Code `.claude-plugin/`, etc.), scans for skills using the
/// appropriate adapter, and records the repo + skills in the manifest.
pub async fn install_skill(source: &str, install_dir: &Path) -> Result<Vec<SkillMetadata>> {
    let (owner, repo) = parse_source(source)?;
    let dir_name = format!("{owner}-{repo}");
    let target = install_dir.join(&dir_name);

    if target.exists() {
        let manifest_path = ManifestStore::default_path()?;
        let store = ManifestStore::new(manifest_path);
        let already_installed = store.with_lock(|manifest| {
            if manifest.find_repo(source).is_some() {
                return Ok(true);
            }
            Ok(false)
        })?;

        if already_installed {
            return InstallSnafu {
                message: format!(
                    "repo directory already exists: {}. Remove it first with `skills remove`.",
                    target.display()
                ),
            }
            .fail();
        }
        // Directory exists but not in manifest — stale leftover, remove it.
        tokio::fs::remove_dir_all(&target).await.context(IoSnafu)?;
    }

    tokio::fs::create_dir_all(install_dir)
        .await
        .context(IoSnafu)?;

    let commit_sha = install_via_http(&owner, &repo, &target).await?;

    // Auto-detect repo format and scan accordingly.
    let format = detect_format(&target);
    let (skills_meta, skill_states) = match format {
        PluginFormat::Skill => scan_repo_skills(&target, install_dir).await?,
        _ => {
            let adapter_result = match scan_with_adapter(&target, format) {
                Some(result) => {
                    let entries = result?;
                    let relative = target
                        .strip_prefix(install_dir)
                        .unwrap_or(&target)
                        .to_string_lossy()
                        .to_string();
                    let meta: Vec<SkillMetadata> =
                        entries.iter().map(|e| e.metadata.clone()).collect();
                    let states: Vec<SkillState> = entries
                        .iter()
                        .map(|e| SkillState {
                            name:          e.metadata.name.clone(),
                            relative_path: relative.clone(),
                            trusted:       false,
                            enabled:       true,
                        })
                        .collect();
                    (meta, states)
                }
                None => (Vec::new(), Vec::new()),
            };

            // Fallback: hybrid repos may use a non-Skill manifest (e.g.
            // marketplace.json) alongside native SKILL.md files. When the
            // format-specific adapter finds nothing, try SKILL.md scanning.
            let (ref meta, _) = adapter_result;
            if meta.is_empty() {
                let (fallback_meta, fallback_states) =
                    scan_repo_skills(&target, install_dir).await?;
                if fallback_meta.is_empty() {
                    adapter_result
                } else {
                    tracing::info!(
                        format = %format,
                        count = fallback_meta.len(),
                        "adapter scan empty, fell back to SKILL.md scanning"
                    );
                    (fallback_meta, fallback_states)
                }
            } else {
                adapter_result
            }
        }
    };

    if skills_meta.is_empty() {
        let _ = tokio::fs::remove_dir_all(&target).await;
        return InstallSnafu {
            message: format!(
                "repository contains no skills (checked {})",
                target.display()
            ),
        }
        .fail();
    }

    // Write manifest under exclusive lock.
    let manifest_path = ManifestStore::default_path()?;
    let store = ManifestStore::new(manifest_path);
    store.with_lock(|manifest| {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        manifest.add_repo(RepoEntry {
            source: format!("{owner}/{repo}"),
            repo_name: dir_name.clone(),
            installed_at_ms: now,
            commit_sha: commit_sha.clone(),
            format,
            skills: skill_states.clone(),
        });
        Ok(())
    })?;

    tracing::info!(count = skills_meta.len(), %source, "installed repo skills");
    Ok(skills_meta)
}

/// Remove a repo: delete directory and manifest entry.
pub async fn remove_repo(source: &str, install_dir: &Path) -> Result<()> {
    let manifest_path = ManifestStore::default_path()?;
    let store = ManifestStore::new(manifest_path);

    let dir = store.with_lock(|manifest| {
        let repo = manifest.find_repo(source).ok_or_else(|| {
            NotFoundSnafu {
                name: format!("repo '{source}' not found in manifest"),
            }
            .build()
        })?;
        let dir = install_dir.join(&repo.repo_name);
        manifest.remove_repo(source);
        Ok(dir)
    })?;

    if dir.exists() {
        tokio::fs::remove_dir_all(&dir).await.context(IoSnafu)?;
    }

    Ok(())
}

/// Install by fetching a tarball from GitHub's API.
async fn install_via_http(owner: &str, repo: &str, target: &Path) -> Result<Option<String>> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/tarball");
    let gh = crate::github::GitHubClient::new();
    let commit_sha = fetch_latest_commit_sha(&gh, owner, repo).await;
    let resp = gh.get(&url, "GitHub tarball download").await?;

    let bytes = resp.bytes().await.context(RequestSnafu)?;

    tokio::fs::create_dir_all(target).await.context(IoSnafu)?;
    let target_owned = target.to_path_buf();
    let owner_owned = owner.to_string();
    let repo_owned = repo.to_string();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let canonical_target = std::fs::canonicalize(&target_owned).context(IoSnafu)?;
        let decoder = flate2::read::GzDecoder::new(&bytes[..]);
        let mut archive = tar::Archive::new(decoder);
        for entry in archive.entries().context(IoSnafu)? {
            let mut entry = entry.context(IoSnafu)?;
            if entry.header().entry_type().is_symlink()
                || entry.header().entry_type().is_hard_link()
            {
                tracing::warn!(owner = %owner_owned, repo = %repo_owned, "skipping symlink/hardlink archive entry");
                continue;
            }

            let path = entry.path().context(IoSnafu)?.into_owned();
            let Some(stripped) = sanitize_archive_path(&path)? else {
                continue;
            };

            let dest = target_owned.join(&stripped);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).context(IoSnafu)?;
                let canonical_parent = std::fs::canonicalize(parent).context(IoSnafu)?;
                if !canonical_parent.starts_with(&canonical_target) {
                    return ArchiveSnafu {
                        message: "archive entry escaped install directory",
                    }
                    .fail();
                }
            }

            if dest.exists() {
                let meta = std::fs::symlink_metadata(&dest).context(IoSnafu)?;
                if meta.file_type().is_symlink() {
                    return ArchiveSnafu {
                        message: "archive entry resolves to symlink destination",
                    }
                    .fail();
                }
            }

            if entry.header().entry_type().is_dir() {
                std::fs::create_dir_all(&dest).context(IoSnafu)?;
                continue;
            }

            entry.unpack(&dest).context(IoSnafu)?;
        }
        Ok(())
    })
    .await
    .context(TaskJoinSnafu)??;

    tracing::info!(%owner, %repo, "installed skill repo via HTTP tarball");
    Ok(commit_sha)
}

/// Best-effort fetch of the latest commit SHA for a repo.
///
/// Returns `None` on any error — the SHA is informational only.
async fn fetch_latest_commit_sha(
    gh: &crate::github::GitHubClient,
    owner: &str,
    repo: &str,
) -> Option<String> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/commits?per_page=1");
    let response = gh.get(&url, "GitHub latest commit SHA").await.ok()?;
    let value: serde_json::Value = response.json().await.ok()?;
    value
        .as_array()?
        .first()?
        .get("sha")?
        .as_str()
        .filter(|sha| sha.len() == 40)
        .map(ToOwned::to_owned)
}

fn sanitize_archive_path(path: &Path) -> Result<Option<PathBuf>> {
    let stripped: PathBuf = path.components().skip(1).collect();
    if stripped.as_os_str().is_empty() {
        return Ok(None);
    }

    for component in stripped.components() {
        match component {
            Component::Normal(_) => {}
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return ArchiveSnafu {
                    message: format!("archive contains unsafe path component: {}", path.display()),
                }
                .fail();
            }
        }
    }

    Ok(Some(stripped))
}

/// Recursively scan a cloned repo for SKILL.md files.
/// Returns `(Vec<SkillMetadata>, Vec<SkillState>)` -- metadata for callers and
/// state entries for the manifest.
async fn scan_repo_skills(
    repo_dir: &Path,
    install_dir: &Path,
) -> Result<(Vec<SkillMetadata>, Vec<SkillState>)> {
    // Check root SKILL.md (single-skill repo).
    let root_skill_md = repo_dir.join("SKILL.md");
    if root_skill_md.is_file() {
        let content = tokio::fs::read_to_string(&root_skill_md)
            .await
            .context(IoSnafu)?;
        let mut meta = parse::parse_metadata(&content, repo_dir)?;
        meta.source = Some(crate::types::SkillSource::Registry);

        let relative = repo_dir
            .strip_prefix(install_dir)
            .unwrap_or(repo_dir)
            .to_string_lossy()
            .to_string();

        let state = SkillState {
            name:          meta.name.clone(),
            relative_path: relative,
            trusted:       false,
            enabled:       true,
        };
        return Ok((vec![meta], vec![state]));
    }

    // Multi-skill: recursively scan for SKILL.md.
    let mut skills_meta = Vec::new();
    let mut skill_states = Vec::new();
    let mut dirs_to_scan = vec![repo_dir.to_path_buf()];

    while let Some(dir) = dirs_to_scan.pop() {
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(e) => e,
            Err(_) => continue,
        };
        while let Some(entry) = entries.next_entry().await.context(IoSnafu)? {
            let subdir = entry.path();
            if !subdir.is_dir() {
                continue;
            }
            let skill_md = subdir.join("SKILL.md");
            if skill_md.is_file() {
                let content = match tokio::fs::read_to_string(&skill_md).await {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::debug!(?skill_md, %e, "skipping unreadable SKILL.md");
                        continue;
                    }
                };
                match parse::parse_metadata(&content, &subdir) {
                    Ok(mut meta) => {
                        meta.source = Some(crate::types::SkillSource::Registry);
                        let relative = subdir
                            .strip_prefix(install_dir)
                            .unwrap_or(&subdir)
                            .to_string_lossy()
                            .to_string();
                        skill_states.push(SkillState {
                            name:          meta.name.clone(),
                            relative_path: relative,
                            trusted:       false,
                            enabled:       true,
                        });
                        skills_meta.push(meta);
                    }
                    Err(e) => {
                        tracing::debug!(?skill_md, %e, "skipping non-conforming SKILL.md");
                    }
                }
            } else {
                dirs_to_scan.push(subdir);
            }
        }
    }

    Ok((skills_meta, skill_states))
}

/// Parse `owner/repo` from a source string.
/// Accepts `owner/repo`, `https://github.com/owner/repo`, or with trailing slash/`.git`.
pub fn parse_source(source: &str) -> Result<(String, String)> {
    let s = source.trim().trim_end_matches('/').trim_end_matches(".git");
    let s = s
        .strip_prefix("https://github.com/")
        .or_else(|| s.strip_prefix("http://github.com/"))
        .or_else(|| s.strip_prefix("github.com/"))
        .unwrap_or(s);
    let parts: Vec<&str> = s.split('/').collect();

    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        return InvalidInputSnafu {
            message: format!(
                "invalid skill source '{}': expected 'owner/repo' or GitHub URL",
                source
            ),
        }
        .fail();
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

/// Get the default installation directory.
pub fn default_install_dir() -> Result<PathBuf> {
    Ok(rara_paths::data_dir().join("installed-skills"))
}
