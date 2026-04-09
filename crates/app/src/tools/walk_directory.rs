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

//! Recursive directory tree traversal.
//!
//! Walks a directory tree up to a configurable depth, returning a flat list of
//! entries with path, type, size, and depth metadata.

use anyhow::Context;
use async_trait::async_trait;
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const MAX_ENTRIES: usize = 2000;
const DEFAULT_MAX_DEPTH: usize = 5;

/// Parameters for the walk-directory tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct WalkDirectoryParams {
    /// Root directory to walk.
    path:      String,
    /// Maximum recursion depth (default 5). 0 = root only.
    max_depth: Option<usize>,
    /// Optional glob pattern to filter file entries (e.g. "*.rs", "*.toml").
    pattern:   Option<String>,
}

/// Single entry in the walk result.
#[derive(Debug, Clone, Serialize)]
pub struct WalkEntry {
    /// Relative path from the root.
    path:       String,
    /// Entry type: "file", "dir", or "symlink".
    #[serde(rename = "type")]
    entry_type: String,
    /// File size in bytes (0 for directories).
    size:       u64,
    /// Nesting depth relative to root (0 = immediate child).
    depth:      usize,
}

/// Result of a directory walk.
#[derive(Debug, Clone, Serialize)]
pub struct WalkDirectoryResult {
    /// Matched entries.
    entries:   Vec<WalkEntry>,
    /// Total number of entries found (may exceed `entries.len()` if truncated).
    total:     usize,
    /// Whether the result was truncated at MAX_ENTRIES.
    truncated: bool,
}

/// Recursively walk a directory tree and return entries with metadata.
#[derive(ToolDef)]
#[tool(
    name = "walk-directory",
    description = "Recursively walk a directory tree. Returns each entry's relative path, type \
                   (file/dir/symlink), size, and depth. Supports max_depth limit and glob pattern \
                   filtering. Maximum 2000 entries. Use this to understand project structure.",
    tier = "deferred",
    read_only,
    concurrency_safe
)]
pub struct WalkDirectoryTool;

impl WalkDirectoryTool {
    /// Create a new instance.
    pub fn new() -> Self { Self }
}

/// Walk a directory tree, returning entries and total count.
///
/// Extracted as a standalone function so it can be unit-tested without
/// constructing a full [`ToolContext`].
fn walk_tree(
    root: &std::path::Path,
    max_depth: usize,
    glob_matcher: Option<&glob::Pattern>,
) -> (Vec<WalkEntry>, usize) {
    let mut entries = Vec::new();
    let mut total = 0usize;

    for entry in walkdir::WalkDir::new(root)
        .max_depth(max_depth)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            // Skip hidden directories (except root).
            e.depth() == 0 || !e.file_name().to_str().map_or(false, |s| s.starts_with('.'))
        })
        .flatten()
    {
        if entry.depth() == 0 {
            continue;
        }

        let file_type = entry.file_type();

        // Apply glob filter to files only (keep dirs for structure).
        if let Some(matcher) = glob_matcher {
            if !file_type.is_dir() {
                let file_name = entry.file_name().to_string_lossy();
                if !matcher.matches(&file_name) {
                    continue;
                }
            }
        }

        total += 1;
        if entries.len() < MAX_ENTRIES {
            let rel_path = entry
                .path()
                .strip_prefix(root)
                .unwrap_or(entry.path())
                .to_string_lossy()
                .into_owned();
            let type_str = if file_type.is_dir() {
                "dir"
            } else if file_type.is_symlink() {
                "symlink"
            } else {
                "file"
            };
            let size = if file_type.is_file() {
                entry.metadata().map(|m| m.len()).unwrap_or(0)
            } else {
                0
            };
            entries.push(WalkEntry {
                path: rel_path,
                entry_type: type_str.to_owned(),
                size,
                depth: entry.depth(),
            });
        }
    }

    (entries, total)
}

#[async_trait]
impl ToolExecute for WalkDirectoryTool {
    type Output = WalkDirectoryResult;
    type Params = WalkDirectoryParams;

    async fn run(
        &self,
        params: WalkDirectoryParams,
        _context: &ToolContext,
    ) -> anyhow::Result<WalkDirectoryResult> {
        let root = {
            let p = std::path::PathBuf::from(&params.path);
            if p.is_absolute() {
                p
            } else {
                rara_paths::workspace_dir().join(p)
            }
        };
        let max_depth = params.max_depth.unwrap_or(DEFAULT_MAX_DEPTH);

        let glob_matcher = params
            .pattern
            .as_deref()
            .map(glob::Pattern::new)
            .transpose()
            .context("invalid glob pattern")?;

        // Run blocking walkdir in spawn_blocking to avoid blocking the runtime.
        let (entries, total) =
            tokio::task::spawn_blocking(move || walk_tree(&root, max_depth, glob_matcher.as_ref()))
                .await
                .context("walkdir task panicked")?;

        Ok(WalkDirectoryResult {
            truncated: total > MAX_ENTRIES,
            entries,
            total,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk_returns_entries() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let (entries, total) = walk_tree(root, 1, None);
        assert!(!entries.is_empty());
        assert!(total > 0);
        assert!(
            entries.iter().any(|e| e.path.contains("Cargo.toml")),
            "expected Cargo.toml in entries: {entries:?}"
        );
    }

    #[test]
    fn walk_filters_by_pattern() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let pattern = glob::Pattern::new("*.rs").expect("valid glob");
        let (entries, _) = walk_tree(root, 3, Some(&pattern));
        // All file entries must match *.rs; dirs are kept for structure.
        assert!(entries.iter().filter(|e| e.entry_type == "file").all(|e| {
            std::path::Path::new(&e.path)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("rs"))
        }));
    }

    #[test]
    fn walk_respects_max_depth() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let (entries, _) = walk_tree(root, 1, None);
        // depth=1 means only immediate children.
        assert!(
            entries.iter().all(|e| e.depth == 1),
            "all entries should be at depth 1"
        );
    }

    #[test]
    fn walk_skips_hidden_dirs() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let (entries, _) = walk_tree(root, 5, None);
        assert!(
            !entries.iter().any(|e| e.path.starts_with('.')),
            "hidden entries should be skipped"
        );
    }
}
