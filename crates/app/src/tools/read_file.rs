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

//! File reading primitive.
//!
//! Reads a file with optional line offset and limit, adds `cat -n` style line
//! number prefixes, and truncates long lines at 2000 characters.
//!
//! When invoked without explicit offset/limit, adaptively pages through the
//! file up to a budget derived from the model's context window size.

use anyhow::Context;
use async_trait::async_trait;
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Maximum total output size in bytes per page (50 KB).
const MAX_OUTPUT_BYTES: usize = 50 * 1024;

/// Maximum characters per line before truncation.
const MAX_LINE_CHARS: usize = 2000;

/// Default maximum number of lines to return per page.
const DEFAULT_LIMIT: usize = 2000;

/// Number of bytes to check for binary detection.
const BINARY_CHECK_BYTES: usize = 1024;

/// Estimated characters per token for budget calculation.
const CHARS_PER_TOKEN: usize = 4;

/// Fraction of context window allocated to a single file read.
const CONTEXT_SHARE: f64 = 0.15;

/// Minimum adaptive paging budget in bytes (50 KB).
const MIN_BUDGET_BYTES: usize = 50 * 1024;

/// Maximum adaptive paging budget in bytes (512 KB).
const MAX_BUDGET_BYTES: usize = 512 * 1024;

/// Maximum number of pages in a single adaptive read.
const MAX_PAGES: usize = 8;

/// Result of reading a single page from already-loaded file content.
struct PageResult {
    /// Formatted output with line number prefixes.
    output:            String,
    /// Number of lines included in this page.
    lines_read:        usize,
    /// Whether there are unread lines beyond this page.
    has_more_lines:    bool,
    /// Whether any line content was truncated (long lines) or the page
    /// hit the byte limit before exhausting `limit` lines.
    content_truncated: bool,
    /// Total number of lines in the file.
    total_lines:       usize,
}

/// Compute the adaptive paging budget from the model's context window size.
fn compute_budget(context_window_tokens: usize) -> usize {
    let raw = (context_window_tokens as f64 * CHARS_PER_TOKEN as f64 * CONTEXT_SHARE) as usize;
    raw.clamp(MIN_BUDGET_BYTES, MAX_BUDGET_BYTES)
}

/// Read a single page from pre-loaded lines.
///
/// `offset` is 1-based. Returns up to `limit` lines starting from `offset`,
/// formatted with `cat -n` style line numbers and bounded by
/// `MAX_OUTPUT_BYTES`.
fn read_page(all_lines: &[&str], offset: usize, limit: usize) -> PageResult {
    let total_lines = all_lines.len();
    let start_idx = (offset - 1).min(total_lines);
    let end_idx = (start_idx + limit).min(total_lines);
    let selected = &all_lines[start_idx..end_idx];

    let mut output = String::new();
    let mut content_truncated = false;
    let mut lines_read = 0;

    for (i, line) in selected.iter().enumerate() {
        let line_no = start_idx + i + 1;
        let display_line = if line.len() > MAX_LINE_CHARS {
            content_truncated = true;
            format!("{}... [truncated]", &line[..MAX_LINE_CHARS])
        } else {
            (*line).to_owned()
        };

        let formatted = format!("{line_no:>6}\t{display_line}\n");

        if output.len() + formatted.len() > MAX_OUTPUT_BYTES {
            content_truncated = true;
            break;
        }
        output.push_str(&formatted);
        lines_read += 1;
    }

    let has_more_lines = (start_idx + lines_read) < total_lines;

    PageResult {
        output,
        lines_read,
        has_more_lines,
        content_truncated,
        total_lines,
    }
}

/// Input parameters for the read-file tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadFileParams {
    /// Absolute path to the file to read.
    file_path: String,
    /// 1-based line number to start reading from (default 1).
    offset:    Option<u64>,
    /// Maximum number of lines to return (default 2000).
    limit:     Option<u64>,
}

/// Typed result returned by the read-file tool.
#[derive(Debug, Clone, Serialize)]
pub struct ReadFileResult {
    /// File content with line number prefixes.
    pub content:     String,
    /// Total number of lines in the file.
    pub total_lines: usize,
    /// Whether the output was truncated.
    pub truncated:   bool,
}

/// Layer 1 primitive: read a file with line numbers.
#[derive(ToolDef)]
#[tool(
    name = "read-file",
    description = "Read a file from the filesystem. Returns content with line number prefixes \
                   (like cat -n). Without offset/limit, adaptively reads up to the context-window \
                   budget (multiple pages auto-stitched). Use offset and limit to read a specific \
                   range. Detects binary files. Long lines are truncated at 2000 characters."
)]
pub struct ReadFileTool;

impl ReadFileTool {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl ToolExecute for ReadFileTool {
    type Output = ReadFileResult;
    type Params = ReadFileParams;

    async fn run(
        &self,
        params: ReadFileParams,
        context: &ToolContext,
    ) -> anyhow::Result<ReadFileResult> {
        let file_path = if std::path::Path::new(&params.file_path).is_absolute() {
            std::path::PathBuf::from(&params.file_path)
        } else {
            rara_paths::workspace_dir().join(&params.file_path)
        };

        let raw_bytes = tokio::fs::read(&file_path)
            .await
            .context(format!("failed to read file {}", file_path.display()))?;

        // Binary detection: check for null bytes in the first BINARY_CHECK_BYTES.
        let check_len = raw_bytes.len().min(BINARY_CHECK_BYTES);
        if raw_bytes[..check_len].contains(&0) {
            return Ok(ReadFileResult {
                content:     "[binary file detected]".to_owned(),
                total_lines: 0,
                truncated:   false,
            });
        }

        let content = String::from_utf8_lossy(&raw_bytes);
        let all_lines: Vec<&str> = content.lines().collect();

        // Single-page mode: agent explicitly specified offset or limit.
        if params.offset.is_some() || params.limit.is_some() {
            let offset = params.offset.map(|v| v.max(1) as usize).unwrap_or(1);
            let limit = params.limit.map(|v| v as usize).unwrap_or(DEFAULT_LIMIT);
            let page = read_page(&all_lines, offset, limit);
            return Ok(ReadFileResult {
                content:     page.output,
                total_lines: page.total_lines,
                truncated:   page.has_more_lines || page.content_truncated,
            });
        }

        // Adaptive paging mode: read multiple pages up to budget.
        let budget = compute_budget(context.context_window_tokens);
        let mut accumulated = String::new();
        let mut page_offset: usize = 1;
        let mut file_fully_read = false;
        let mut any_content_truncated = false;
        let mut total_lines = 0;

        for _ in 0..MAX_PAGES {
            let page = read_page(&all_lines, page_offset, DEFAULT_LIMIT);
            total_lines = page.total_lines;
            any_content_truncated |= page.content_truncated;
            accumulated.push_str(&page.output);

            if !page.has_more_lines {
                file_fully_read = true;
                break;
            }

            if accumulated.len() >= budget {
                break;
            }

            page_offset += page.lines_read;
        }

        if !file_fully_read {
            let last_line_no = accumulated
                .lines()
                .last()
                .and_then(|l| l.trim_start().split('\t').next())
                .and_then(|n| n.trim().parse::<usize>().ok())
                .unwrap_or(0);
            accumulated.push_str(&format!(
                "\n[Showing lines 1-{last_line_no} of {total_lines}. Use offset={next} to \
                 continue.]\n",
                next = last_line_no + 1,
            ));
        }

        Ok(ReadFileResult {
            content: accumulated,
            total_lines,
            truncated: !file_fully_read || any_content_truncated,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── compute_budget ───────────────────────────────────────────────

    #[test]
    fn budget_clamps_to_min() {
        // Very small context window → floor at 50 KB.
        assert_eq!(compute_budget(1_000), MIN_BUDGET_BYTES);
    }

    #[test]
    fn budget_clamps_to_max() {
        // Huge context window → ceiling at 512 KB.
        assert_eq!(compute_budget(10_000_000), MAX_BUDGET_BYTES);
    }

    #[test]
    fn budget_200k_model() {
        // 200_000 * 4 * 0.15 = 120_000
        assert_eq!(compute_budget(200_000), 120_000);
    }

    #[test]
    fn budget_128k_model() {
        // 128_000 * 4 * 0.15 = 76_800
        assert_eq!(compute_budget(128_000), 76_800);
    }

    #[test]
    fn budget_zero_tokens_clamps_to_min() {
        assert_eq!(compute_budget(0), MIN_BUDGET_BYTES);
    }

    // ── read_page basics ─────────────────────────────────────────────

    fn sample_lines(n: usize) -> Vec<String> { (0..n).map(|i| format!("line {i}")).collect() }

    fn as_str_slice(v: &[String]) -> Vec<&str> { v.iter().map(|s| s.as_str()).collect() }

    #[test]
    fn read_page_small_file() {
        let lines = sample_lines(5);
        let refs = as_str_slice(&lines);
        let page = read_page(&refs, 1, 2000);

        assert_eq!(page.lines_read, 5);
        assert_eq!(page.total_lines, 5);
        assert!(!page.has_more_lines);
        assert!(!page.content_truncated);
    }

    #[test]
    fn read_page_with_offset() {
        let lines = sample_lines(10);
        let refs = as_str_slice(&lines);
        let page = read_page(&refs, 4, 3);

        assert_eq!(page.lines_read, 3);
        assert_eq!(page.total_lines, 10);
        assert!(page.has_more_lines);
        // Output should start at line 4.
        assert!(page.output.contains("     4\t"));
    }

    #[test]
    fn read_page_offset_beyond_eof() {
        let lines = sample_lines(5);
        let refs = as_str_slice(&lines);
        let page = read_page(&refs, 100, 10);

        assert_eq!(page.lines_read, 0);
        assert!(!page.has_more_lines);
        assert!(page.output.is_empty());
    }

    #[test]
    fn read_page_limit_exceeds_remaining() {
        let lines = sample_lines(5);
        let refs = as_str_slice(&lines);
        let page = read_page(&refs, 3, 100);

        assert_eq!(page.lines_read, 3); // lines 3, 4, 5
        assert!(!page.has_more_lines);
    }

    // ── truncation flag separation ───────────────────────────────────

    #[test]
    fn long_line_sets_content_truncated_not_has_more() {
        // A single line longer than MAX_LINE_CHARS in a 1-line file.
        let long = "x".repeat(MAX_LINE_CHARS + 500);
        let refs = vec![long.as_str()];
        let page = read_page(&refs, 1, 2000);

        assert_eq!(page.lines_read, 1);
        assert!(!page.has_more_lines, "file fully read, no more lines");
        assert!(page.content_truncated, "long line was clipped");
        assert!(page.output.contains("... [truncated]"));
    }

    #[test]
    fn has_more_lines_true_when_limit_reached() {
        let lines = sample_lines(100);
        let refs = as_str_slice(&lines);
        let page = read_page(&refs, 1, 10);

        assert_eq!(page.lines_read, 10);
        assert!(page.has_more_lines);
        assert!(!page.content_truncated);
    }

    #[test]
    fn both_flags_independent() {
        // File with a long line and more lines beyond the limit.
        let mut lines: Vec<String> = sample_lines(20);
        lines[5] = "y".repeat(MAX_LINE_CHARS + 100);
        let refs = as_str_slice(&lines);
        let page = read_page(&refs, 1, 10);

        assert!(page.has_more_lines, "20 lines, limit 10");
        assert!(page.content_truncated, "line 5 exceeds MAX_LINE_CHARS");
    }

    // ── line number formatting ───────────────────────────────────────

    #[test]
    fn output_has_cat_n_format() {
        let lines = sample_lines(3);
        let refs = as_str_slice(&lines);
        let page = read_page(&refs, 1, 10);

        let output_lines: Vec<&str> = page.output.lines().collect();
        assert_eq!(output_lines.len(), 3);
        assert!(output_lines[0].starts_with("     1\t"));
        assert!(output_lines[2].starts_with("     3\t"));
    }
}
