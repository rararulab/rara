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

use std::path::Path;

use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use snafu::ResultExt;
use tracing::{debug, info, warn};

use crate::error::{ArchiveSnafu, InstallSnafu, IoSnafu, RequestSnafu};

/// Maximum retry attempts for API calls (including the first try).
const MAX_RETRIES: u32 = 3;

/// Base delay in milliseconds for exponential backoff.
const BASE_DELAY_MS: u64 = 1_500;

/// Maximum delay cap in milliseconds.
const MAX_DELAY_MS: u64 = 15_000;

/// Default ClawHub API base URL.
const DEFAULT_BASE_URL: &str = "https://clawhub.ai/api/v1";

/// Client for the ClawHub marketplace (clawhub.ai).
pub struct ClawhubClient {
    base_url: String,
    client:   reqwest::Client,
}

impl Default for ClawhubClient {
    fn default() -> Self { Self::new() }
}

impl ClawhubClient {
    /// Create a new ClawHub client with default settings.
    pub fn new() -> Self { Self::with_url(DEFAULT_BASE_URL) }

    /// Create a ClawHub client with a custom API URL.
    pub fn with_url(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_else(|e| {
                    warn!(error = %e, "failed to build ClawHub HTTP client with timeout, falling back to default");
                    reqwest::Client::default()
                }),
        }
    }

    /// Issue a GET request with automatic retry on 429 and 5xx.
    async fn get_with_retry(
        &self,
        url: &str,
        context: &str,
    ) -> Result<reqwest::Response, crate::error::SkillError> {
        let mut next_delay_ms: Option<u64> = None;

        for attempt in 0..MAX_RETRIES {
            if let Some(delay_ms) = next_delay_ms.take() {
                debug!(attempt, delay_ms, context, "retrying ClawHub request");
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }

            let result = self
                .client
                .get(url)
                .header("User-Agent", "rara-clawhub/0.1")
                .send()
                .await;

            match result {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        return Ok(resp);
                    }
                    if status.as_u16() == 429 || status.is_server_error() {
                        if attempt + 1 < MAX_RETRIES {
                            // Prefer Retry-After header, fall back to exponential backoff.
                            let backoff = BASE_DELAY_MS
                                .saturating_mul(1u64 << (attempt + 1).min(5))
                                .min(MAX_DELAY_MS);
                            let delay = resp
                                .headers()
                                .get("retry-after")
                                .and_then(|v| v.to_str().ok())
                                .and_then(|v| v.parse::<u64>().ok())
                                .map(|secs| (secs * 1000).min(MAX_DELAY_MS))
                                .unwrap_or(backoff);
                            next_delay_ms = Some(delay);
                            continue;
                        }
                        return InstallSnafu {
                            message: format!(
                                "{context} returned {status} after {MAX_RETRIES} attempts"
                            ),
                        }
                        .fail();
                    }
                    return InstallSnafu {
                        message: format!("{context} returned {status}"),
                    }
                    .fail();
                }
                Err(e) => {
                    if attempt + 1 >= MAX_RETRIES {
                        return InstallSnafu {
                            message: format!("{context} failed after {MAX_RETRIES} attempts: {e}"),
                        }
                        .fail();
                    }
                    let backoff = BASE_DELAY_MS
                        .saturating_mul(1u64 << (attempt + 1).min(5))
                        .min(MAX_DELAY_MS);
                    next_delay_ms = Some(backoff);
                    warn!(attempt, context, error = %e, "ClawHub request failed, will retry");
                }
            }
        }
        unreachable!()
    }

    /// Search for skills on ClawHub using vector/semantic search.
    ///
    /// Uses `GET /api/v1/search?q=...&limit=...`.
    pub async fn search(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<ClawhubSearchResponse, crate::error::SkillError> {
        let url = reqwest::Url::parse_with_params(
            &format!("{}/search", self.base_url),
            &[("q", query), ("limit", &limit.min(50).to_string())],
        )
        .with_whatever_context::<_, _, crate::error::SkillError>(|e| {
            format!("invalid ClawHub search URL: {e}")
        })?;
        let resp = self.get_with_retry(url.as_str(), "ClawHub search").await?;
        resp.json::<ClawhubSearchResponse>()
            .await
            .context(RequestSnafu)
    }

    /// Browse skills by sort order.
    ///
    /// Uses `GET /api/v1/skills?limit=...&sort=...`.
    pub async fn browse(
        &self,
        sort: ClawhubSort,
        limit: u32,
    ) -> Result<ClawhubBrowseResponse, crate::error::SkillError> {
        let url = reqwest::Url::parse_with_params(
            &format!("{}/skills", self.base_url),
            &[
                ("limit", limit.min(50).to_string()),
                ("sort", sort.as_str().to_string()),
            ],
        )
        .with_whatever_context::<_, _, crate::error::SkillError>(|e| {
            format!("invalid ClawHub browse URL: {e}")
        })?;
        let resp = self.get_with_retry(url.as_str(), "ClawHub browse").await?;
        resp.json::<ClawhubBrowseResponse>()
            .await
            .context(RequestSnafu)
    }

    /// Get detailed info about a specific skill.
    ///
    /// Uses `GET /api/v1/skills/{slug}`.
    pub async fn get_skill(
        &self,
        slug: &str,
    ) -> Result<ClawhubSkillDetail, crate::error::SkillError> {
        let url = format!("{}/skills/{}", self.base_url, percent_encode_path(slug));
        let resp = self.get_with_retry(&url, "ClawHub skill detail").await?;
        resp.json::<ClawhubSkillDetail>()
            .await
            .context(RequestSnafu)
    }

    /// Download and install a skill from ClawHub into the target directory.
    ///
    /// Uses `GET /api/v1/download?slug=...` to download a zip, extracts it
    /// into `{install_dir}/{slug}/`, then registers in the skills manifest.
    pub async fn install(
        &self,
        slug: &str,
        install_dir: &Path,
    ) -> Result<ClawhubInstallResult, crate::error::SkillError> {
        let skill_dir = install_dir.join(slug);
        let manifest_path = crate::manifest::ManifestStore::default_path()?;
        let store = crate::manifest::ManifestStore::new(manifest_path);
        let source = format!("clawhub:{slug}");

        // Conflict check: if directory exists and is tracked in manifest, reject.
        // If directory exists but is NOT in manifest (stale), remove it first.
        if skill_dir.exists() {
            let manifest = store.load()?;
            if manifest.find_repo(&source).is_some() {
                return InstallSnafu {
                    message: format!(
                        "ClawHub skill '{slug}' is already installed. Remove it first with \
                         `skills remove`."
                    ),
                }
                .fail();
            }
            tokio::fs::remove_dir_all(&skill_dir)
                .await
                .context(IoSnafu)?;
        }

        // Fetch detail first to validate slug exists and get version before
        // downloading.
        let detail = self.get_skill(slug).await?;
        let version = detail.latest_version.map(|v| v.version).unwrap_or_default();

        let url = reqwest::Url::parse_with_params(
            &format!("{}/download", self.base_url),
            &[("slug", slug)],
        )
        .with_whatever_context::<_, _, crate::error::SkillError>(|e| {
            format!("invalid ClawHub download URL: {e}")
        })?;
        info!(slug, "downloading skill from ClawHub");

        let resp = self
            .get_with_retry(url.as_str(), "ClawHub download")
            .await?;
        let bytes = resp.bytes().await.context(RequestSnafu)?;

        std::fs::create_dir_all(&skill_dir).context(IoSnafu)?;

        // Use a helper closure to ensure cleanup on failure.
        let result = self
            .install_inner(
                slug,
                install_dir,
                &skill_dir,
                &store,
                &source,
                &bytes,
                &version,
            )
            .await;

        if result.is_err() {
            let _ = tokio::fs::remove_dir_all(&skill_dir).await;
        }

        result
    }

    async fn install_inner(
        &self,
        slug: &str,
        install_dir: &Path,
        skill_dir: &Path,
        store: &crate::manifest::ManifestStore,
        source: &str,
        bytes: &[u8],
        version: &str,
    ) -> Result<ClawhubInstallResult, crate::error::SkillError> {
        let is_zip = bytes.len() >= 4 && bytes[0] == 0x50 && bytes[1] == 0x4b;

        if is_zip {
            extract_zip(bytes, skill_dir)?;
            info!(slug, "extracted ClawHub skill zip");
        } else {
            // Non-zip content: write directly as SKILL.md.
            std::fs::write(skill_dir.join("SKILL.md"), bytes).context(IoSnafu)?;
        }

        let skills = scan_skill_files(install_dir, skill_dir, slug);
        if skills.is_empty() {
            return InstallSnafu {
                message: format!("installed ClawHub package '{slug}' contains no SKILL.md"),
            }
            .fail();
        }

        // Register in manifest under exclusive lock.
        store.with_lock(|manifest| {
            manifest.remove_repo(source);
            manifest.add_repo(crate::types::RepoEntry {
                source:          source.to_string(),
                repo_name:       slug.to_string(),
                installed_at_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
                commit_sha:      None,
                format:          crate::formats::PluginFormat::Skill,
                skills:          skills.clone(),
            });
            Ok(())
        })?;

        let skill_names: Vec<String> = skills.iter().map(|s| s.name.clone()).collect();
        info!(slug, skills = skill_names.len(), "installed ClawHub skill");

        Ok(ClawhubInstallResult {
            slug:         slug.to_string(),
            version:      version.to_string(),
            skills_count: skill_names.len(),
            skills:       skill_names,
        })
    }
}

/// Extract a zip archive into `dest_dir` with path traversal protection.
///
/// Mirrors the security checks in `install.rs`: canonicalize + starts_with
/// to prevent symlink attacks and directory escape.
fn extract_zip(bytes: &[u8], dest_dir: &Path) -> crate::error::Result<()> {
    let canonical_dest = std::fs::canonicalize(dest_dir).context(IoSnafu)?;
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor)
        .with_whatever_context::<_, _, crate::error::SkillError>(|e| {
            format!("failed to read zip: {e}")
        })?;

    for i in 0..archive.len() {
        let mut file = match archive.by_index(i) {
            Ok(f) => f,
            Err(e) => {
                warn!(index = i, error = %e, "skipping zip entry");
                continue;
            }
        };

        // enclosed_name() filters out `..` components.
        let Some(enclosed_name) = file.enclosed_name() else {
            warn!("skipping zip entry with unsafe path");
            continue;
        };

        // Skip symlinks — same policy as tarball extraction in install.rs.
        if file.is_symlink() {
            warn!(name = %enclosed_name.display(), "skipping symlink in zip");
            continue;
        }

        let out_path = dest_dir.join(enclosed_name);

        if file.is_dir() {
            std::fs::create_dir_all(&out_path).context(IoSnafu)?;
            continue;
        }

        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent).context(IoSnafu)?;
            // Verify the resolved parent hasn't escaped the destination.
            let canonical_parent = std::fs::canonicalize(parent).context(IoSnafu)?;
            if !canonical_parent.starts_with(&canonical_dest) {
                return ArchiveSnafu {
                    message: "zip entry escaped install directory",
                }
                .fail();
            }
        }

        // Refuse to overwrite symlinks — prevents symlink-following attacks.
        if out_path.exists() {
            let meta = std::fs::symlink_metadata(&out_path).context(IoSnafu)?;
            if meta.file_type().is_symlink() {
                return ArchiveSnafu {
                    message: "zip entry resolves to symlink destination",
                }
                .fail();
            }
        }

        let mut out_file = std::fs::File::create(&out_path).context(IoSnafu)?;
        std::io::copy(&mut file, &mut out_file).context(IoSnafu)?;
    }

    Ok(())
}

/// Percent-encode a slug for use in a single URL path segment.
///
/// Uses the `PATH_SEGMENT` encode set which encodes everything except
/// unreserved characters and sub-delimiters (but encodes `/`, `?`, `#`).
fn percent_encode_path(s: &str) -> String {
    // NON_ALPHANUMERIC encodes everything except [A-Za-z0-9], which is safe
    // for path segments (over-encodes but never under-encodes).
    utf8_percent_encode(s, NON_ALPHANUMERIC).to_string()
}

/// Recursively scan a directory for SKILL.md files and return SkillState
/// entries.
///
/// Mirrors the recursive walk in `install.rs::scan_repo_skills`: if a directory
/// contains SKILL.md it is registered; otherwise its children are scanned.
fn scan_skill_files(install_dir: &Path, dir: &Path, slug: &str) -> Vec<crate::types::SkillState> {
    let mut skills = Vec::new();

    // Check root-level SKILL.md first.
    if has_skill_md(dir) {
        let relative = dir
            .strip_prefix(install_dir)
            .unwrap_or(dir)
            .to_string_lossy()
            .to_string();
        skills.push(crate::types::SkillState {
            name:          slug.to_string(),
            relative_path: relative,
            trusted:       false,
            enabled:       false,
        });
    }

    // Recursively scan subdirectories.
    let mut dirs_to_scan = vec![dir.to_path_buf()];
    while let Some(current) = dirs_to_scan.pop() {
        let entries = match std::fs::read_dir(&current) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if has_skill_md(&path) {
                let sub_name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");
                let relative = path
                    .strip_prefix(install_dir)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_string();
                skills.push(crate::types::SkillState {
                    name:          format!("{slug}:{sub_name}"),
                    relative_path: relative,
                    trusted:       false,
                    enabled:       false,
                });
            } else {
                // No SKILL.md here, keep scanning deeper.
                dirs_to_scan.push(path);
            }
        }
    }

    skills
}

fn has_skill_md(dir: &Path) -> bool {
    dir.join("SKILL.md").exists() || dir.join("skill.md").exists()
}

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

/// Result of installing a skill from ClawHub.
#[derive(Debug, Clone, Serialize)]
pub struct ClawhubInstallResult {
    pub slug:         String,
    pub version:      String,
    pub skills_count: usize,
    pub skills:       Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_response_deserializes() {
        let json = r#"{
            "results": [{
                "score": 3.71,
                "slug": "github",
                "displayName": "Github",
                "summary": "Interact with GitHub using the gh CLI.",
                "version": "1.0.0",
                "updatedAt": 1771777539580
            }]
        }"#;
        let resp: ClawhubSearchResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.results.len(), 1);
        assert_eq!(resp.results[0].slug, "github");
        assert!(resp.results[0].score > 3.0);
    }

    #[test]
    fn browse_response_deserializes() {
        let json = r#"{
            "items": [{
                "slug": "sonoscli",
                "displayName": "Sonoscli",
                "summary": "Control Sonos speakers.",
                "tags": {"latest": "1.0.0"},
                "stats": {
                    "downloads": 19736,
                    "installsAllTime": 455,
                    "installsCurrent": 437,
                    "stars": 15
                },
                "createdAt": 1767545381030,
                "updatedAt": 1771777535889,
                "latestVersion": {
                    "version": "1.0.0",
                    "createdAt": 1767545381030,
                    "changelog": ""
                }
            }],
            "nextCursor": null
        }"#;
        let resp: ClawhubBrowseResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.items.len(), 1);
        assert_eq!(resp.items[0].slug, "sonoscli");
        assert_eq!(resp.items[0].stats.downloads, 19736);
        assert_eq!(resp.items[0].stats.stars, 15);
    }

    #[test]
    fn detail_response_deserializes() {
        let json = r#"{
            "skill": {
                "slug": "gifgrep",
                "displayName": "GifGrep",
                "summary": "Search GIFs.",
                "stats": { "downloads": 100, "stars": 5 },
                "createdAt": 0,
                "updatedAt": 0
            },
            "latestVersion": { "version": "1.2.3", "createdAt": 0, "changelog": "fix" },
            "owner": { "handle": "steipete", "displayName": "Peter" }
        }"#;
        let detail: ClawhubSkillDetail = serde_json::from_str(json).unwrap();
        assert_eq!(detail.skill.slug, "gifgrep");
        assert_eq!(detail.latest_version.unwrap().version, "1.2.3");
        assert_eq!(detail.owner.unwrap().handle.unwrap(), "steipete");
    }

    #[test]
    fn percent_encode_path_handles_special_chars() {
        assert_eq!(percent_encode_path("hello-world"), "hello%2Dworld");
        assert_eq!(percent_encode_path("foo/bar"), "foo%2Fbar"); // slashes encoded for path segment
        assert_eq!(percent_encode_path("a b"), "a%20b");
        assert_eq!(percent_encode_path("simple"), "simple");
    }
}
