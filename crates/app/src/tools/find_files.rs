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

//! Glob-based file discovery primitive.
//!
//! Uses the `ignore` crate for gitignore-aware file walking and the `glob`
//! crate for pattern matching. No external process dependency.

use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};

use anyhow::Context;
use async_trait::async_trait;
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const DEFAULT_LIMIT: usize = 500;

/// Input parameters for the find-files tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FindFilesParams {
    /// Glob pattern to match files.
    pattern: String,
    /// Directory to search in (default '.').
    path:    Option<String>,
    /// Maximum number of results (default 500).
    limit:   Option<u64>,
}

/// Typed result returned by the find-files tool.
#[derive(Debug, Clone, Serialize)]
pub struct FindFilesResult {
    /// Matched file paths.
    pub files:       Vec<String>,
    /// Total number of files found (before limiting).
    pub total_found: usize,
    /// Whether the result was truncated at the limit.
    pub truncated:   bool,
}

/// Layer 1 primitive: find files matching a glob pattern.
#[derive(ToolDef)]
#[tool(
    name = "find-files",
    description = "Find files by glob pattern, sorted by modification time; respects .gitignore.",
    read_only,
    concurrency_safe
)]
pub struct FindFilesTool;
impl FindFilesTool {
    /// Create a new instance.
    pub fn new() -> Self { Self }
}

#[async_trait]
impl ToolExecute for FindFilesTool {
    type Output = FindFilesResult;
    type Params = FindFilesParams;

    async fn run(
        &self,
        params: FindFilesParams,
        _context: &ToolContext,
    ) -> anyhow::Result<FindFilesResult> {
        let workspace = rara_paths::workspace_dir();
        let path = params.path.as_deref().unwrap_or(".");
        let resolved_path = if Path::new(path).is_absolute() {
            PathBuf::from(path)
        } else {
            workspace.join(path)
        };
        let limit = params.limit.map(|v| v as usize).unwrap_or(DEFAULT_LIMIT);
        let pattern = params.pattern.clone();

        tokio::task::spawn_blocking(move || find_files_in_process(&pattern, &resolved_path, limit))
            .await
            .context("find-files task panicked")?
    }
}

/// Perform in-process file discovery using `ignore::WalkBuilder` +
/// `glob::Pattern`.
fn find_files_in_process(
    pattern: &str,
    search_root: &Path,
    limit: usize,
) -> anyhow::Result<FindFilesResult> {
    let glob_pattern = glob::Pattern::new(pattern).context("invalid glob pattern")?;

    let walker = ignore::WalkBuilder::new(search_root)
        .hidden(false) // include dotfiles to match old `find` behavior
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build();

    // Collect matching files with their modification times for sorting.
    let mut matched: Vec<(String, SystemTime)> = walker
        .flatten()
        .filter(|entry| entry.file_type().map_or(false, |ft| ft.is_file()))
        .filter(|entry| {
            let file_name = entry.file_name().to_string_lossy();
            // Try matching against just the file name first, then the full
            // relative path — this mirrors the behavior of shell glob matching.
            glob_pattern.matches(&file_name)
                || entry
                    .path()
                    .strip_prefix(search_root)
                    .ok()
                    .map(|rel| glob_pattern.matches_path(rel))
                    .unwrap_or(false)
        })
        .map(|entry| {
            let path_str = entry.path().to_string_lossy().into_owned();
            let mtime = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            (path_str, mtime)
        })
        .collect();

    // Sort by modification time, newest first.
    matched.sort_by(|a, b| b.1.cmp(&a.1));

    let total_found = matched.len();
    let truncated = total_found > limit;
    let files: Vec<String> = matched.into_iter().take(limit).map(|(p, _)| p).collect();

    Ok(FindFilesResult {
        files,
        total_found,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_files_discovers_rs_files() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let result = find_files_in_process("*.rs", &root, 100).expect("should succeed");
        assert!(!result.files.is_empty());
        assert!(
            result.files.iter().all(|f| std::path::Path::new(f)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("rs"))),
            "all files should be .rs"
        );
    }

    #[test]
    fn find_files_respects_limit() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let result = find_files_in_process("*.rs", &root, 2).expect("should succeed");
        assert!(result.files.len() <= 2);
        if result.total_found > 2 {
            assert!(result.truncated);
        }
    }

    #[test]
    fn find_files_no_matches() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let result =
            find_files_in_process("*.nonexistent_extension", &root, 100).expect("should succeed");
        assert!(result.files.is_empty());
        assert_eq!(result.total_found, 0);
    }
}
