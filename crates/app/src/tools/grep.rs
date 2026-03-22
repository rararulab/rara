// Copyright 2025 Crrow
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

//! In-process regex search primitive.
//!
//! Uses the `ignore` crate for gitignore-aware file walking and the `regex`
//! crate for pattern matching. No external process dependency (rg/grep).

use std::{
    collections::HashSet,
    fmt::Write as _,
    io::{BufRead, Read as _},
    path::{Path, PathBuf},
};

use anyhow::Context;
use async_trait::async_trait;
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Maximum output size in bytes (50 KB).
const MAX_OUTPUT_BYTES: usize = 50 * 1024;

/// Maximum number of matches to return.
const MAX_MATCHES: usize = 100;

/// Input parameters for the grep tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GrepParams {
    /// Regex pattern to search for.
    pattern:     String,
    /// File or directory to search in (default '.').
    path:        Option<String>,
    /// Glob pattern to filter files (e.g. '*.rs', '*.{ts,tsx}').
    glob:        Option<String>,
    /// Number of context lines to show around each match (default 0).
    context:     Option<u64>,
    /// Enable case-insensitive search (default false).
    ignore_case: Option<bool>,
}

/// Typed result returned by the grep tool.
#[derive(Debug, Clone, Serialize)]
pub struct GrepResult {
    /// Matching lines output.
    pub matches:     String,
    /// Number of matching lines.
    pub match_count: usize,
    /// Whether the output was truncated.
    pub truncated:   bool,
}

/// Layer 1 primitive: regex search across files.
#[derive(ToolDef)]
#[tool(
    name = "grep",
    description = "Search file contents using a regex pattern. Supports file type filtering with \
                   glob patterns, context lines, and case-insensitive search. Respects \
                   .gitignore. Output is truncated to 50KB / 100 matches."
)]
pub struct GrepTool;

impl GrepTool {
    /// Create a new instance.
    pub fn new() -> Self { Self }
}

#[async_trait]
impl ToolExecute for GrepTool {
    type Output = GrepResult;
    type Params = GrepParams;

    async fn run(&self, params: GrepParams, _context: &ToolContext) -> anyhow::Result<GrepResult> {
        let workspace = rara_paths::workspace_dir();
        let raw_path = params.path.as_deref().unwrap_or(".");
        let resolved = if Path::new(raw_path).is_absolute() {
            PathBuf::from(raw_path)
        } else {
            workspace.join(raw_path)
        };
        let ignore_case = params.ignore_case.unwrap_or(false);
        let context_lines = params.context.unwrap_or(0) as usize;
        let pattern = params.pattern.clone();
        let glob_filter = params.glob.clone();

        tokio::task::spawn_blocking(move || {
            grep_in_process(
                &pattern,
                &resolved,
                glob_filter.as_deref(),
                context_lines,
                ignore_case,
            )
        })
        .await
        .context("grep task panicked")?
    }
}

/// Perform an in-process grep using `ignore::WalkBuilder` + `regex::Regex`.
fn grep_in_process(
    pattern: &str,
    search_path: &Path,
    glob_filter: Option<&str>,
    context: usize,
    ignore_case: bool,
) -> anyhow::Result<GrepResult> {
    let re = regex::RegexBuilder::new(pattern)
        .case_insensitive(ignore_case)
        .build()
        .context("invalid regex pattern")?;

    let glob_matcher = glob_filter
        .map(|g| globset::Glob::new(g).map(|gb| gb.compile_matcher()))
        .transpose()
        .context("invalid glob pattern")?;

    // Collect matching file paths first so we can sort them for deterministic
    // output (ignore::WalkBuilder yields in arbitrary order).
    let mut files: Vec<PathBuf> = Vec::new();
    let walker = ignore::WalkBuilder::new(search_path)
        .hidden(true) // skip hidden files
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build();

    for entry in walker.flatten() {
        if !entry.file_type().map_or(false, |ft| ft.is_file()) {
            continue;
        }
        if let Some(ref matcher) = glob_matcher {
            // Match against both filename and relative path so patterns like
            // `src/**/*.rs` work the same as ripgrep's `--glob`.
            let file_name = entry.file_name().to_string_lossy();
            let rel_path = entry
                .path()
                .strip_prefix(search_path)
                .unwrap_or(entry.path());
            if !matcher.is_match(file_name.as_ref())
                && !matcher.is_match(rel_path.to_string_lossy().as_ref())
            {
                continue;
            }
        }
        files.push(entry.into_path());
    }
    files.sort();

    let mut output = String::new();
    let mut match_count: usize = 0;
    let mut truncated = false;

    // Strip the search root from output paths for cleaner display.
    let strip_base = if search_path.is_dir() {
        search_path
    } else {
        search_path.parent().unwrap_or(search_path)
    };

    'outer: for file_path in &files {
        // Detect binary content before reading the full file: check the first
        // 512 bytes for NUL bytes to avoid loading large binaries into memory.
        let mut file = match std::fs::File::open(file_path) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let mut header = [0u8; 512];
        let header_len = file.read(&mut header).unwrap_or(0);
        if header[..header_len].contains(&0) {
            continue;
        }

        // Re-open for line-by-line reading (seek would skip buffered data).
        let file = match std::fs::File::open(file_path) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let reader = std::io::BufReader::new(file);
        let lines: Vec<String> = reader.lines().map_while(Result::ok).collect();

        let rel_path = file_path
            .strip_prefix(strip_base)
            .unwrap_or(file_path)
            .to_string_lossy();

        // Find matching line indices using a HashSet for O(1) lookup during
        // context line emission.
        let match_indices: HashSet<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| re.is_match(line))
            .map(|(i, _)| i)
            .collect();

        if match_indices.is_empty() {
            continue;
        }

        // Sorted copy for building contiguous ranges.
        let mut sorted_indices: Vec<usize> = match_indices.iter().copied().collect();
        sorted_indices.sort_unstable();

        // Build ranges of lines to display (match + context).
        let ranges = build_context_ranges(&sorted_indices, context, lines.len());
        let mut prev_range_end: Option<usize> = None;

        for range in &ranges {
            // Insert separator between non-contiguous ranges.
            if let Some(prev_end) = prev_range_end {
                if range.start > prev_end {
                    let _ = writeln!(output, "--");
                }
            }
            prev_range_end = Some(range.end);

            for (idx, line) in lines.iter().enumerate().take(range.end).skip(range.start) {
                if match_indices.contains(&idx) {
                    match_count += 1;
                    if match_count > MAX_MATCHES {
                        truncated = true;
                        break 'outer;
                    }
                }

                let _ = writeln!(output, "{}:{}:{}", rel_path, idx + 1, line);

                if output.len() > MAX_OUTPUT_BYTES {
                    truncated = true;
                    break 'outer;
                }
            }
        }
    }

    if truncated && output.len() > MAX_OUTPUT_BYTES {
        let safe_end = output.floor_char_boundary(MAX_OUTPUT_BYTES);
        output.truncate(safe_end);
        output.push_str("... [truncated]");
    }

    Ok(GrepResult {
        matches: output,
        match_count,
        truncated,
    })
}

/// Build merged, sorted ranges of `[start..end)` for context display.
fn build_context_ranges(
    match_indices: &[usize],
    context: usize,
    total_lines: usize,
) -> Vec<std::ops::Range<usize>> {
    let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
    for &idx in match_indices {
        let start = idx.saturating_sub(context);
        let end = (idx + context + 1).min(total_lines);
        // Merge with previous range if overlapping or adjacent.
        if let Some(last) = ranges.last_mut() {
            if start <= last.end {
                last.end = last.end.max(end);
                continue;
            }
        }
        ranges.push(start..end);
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grep_finds_matches_in_own_source() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tools/grep.rs");
        let result =
            grep_in_process("GrepTool", &src, None, 0, false).expect("grep should succeed");
        assert!(result.match_count > 0);
        assert!(result.matches.contains("GrepTool"));
    }

    #[test]
    fn grep_case_insensitive() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tools/grep.rs");
        let result = grep_in_process("greptool", &src, None, 0, true).expect("grep should succeed");
        assert!(
            result.match_count > 0,
            "case-insensitive search should find matches"
        );
    }

    #[test]
    fn grep_with_glob_filter() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let result =
            grep_in_process("ToolDef", &dir, Some("*.rs"), 0, false).expect("grep should succeed");
        assert!(result.match_count > 0);
    }

    #[test]
    fn grep_with_context_lines() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tools/grep.rs");
        let result =
            grep_in_process("MAX_OUTPUT_BYTES", &src, None, 2, false).expect("grep should succeed");
        // Context lines produce more output lines than matches.
        let output_lines: usize = result.matches.lines().filter(|l| *l != "--").count();
        assert!(output_lines > result.match_count);
    }

    #[test]
    fn grep_invalid_regex_returns_error() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR"));
        let result = grep_in_process("[invalid", src, None, 0, false);
        assert!(result.is_err());
    }

    #[test]
    fn build_ranges_merges_overlapping() {
        let ranges = build_context_ranges(&[5, 7], 2, 20);
        // [3..8) and [5..10) should merge into [3..10).
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0], 3..10);
    }

    #[test]
    fn build_ranges_keeps_separate() {
        let ranges = build_context_ranges(&[2, 10], 1, 20);
        assert_eq!(ranges.len(), 2);
    }
}
