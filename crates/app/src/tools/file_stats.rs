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

//! File statistics primitive.
//!
//! Returns line count, byte size, and character count for one or more files.
//! Equivalent to `wc -l` but structured for LLM consumption.

use anyhow::Context;
use async_trait::async_trait;
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Parameters for the file-stats tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FileStatsParams {
    /// List of file paths to stat.
    paths: Vec<String>,
}

/// Statistics for a single file.
#[derive(Debug, Clone, Serialize)]
pub struct FileStat {
    /// Resolved absolute path.
    path:  String,
    /// Number of lines (equivalent to `wc -l`).
    lines: usize,
    /// File size in bytes (from filesystem metadata).
    bytes: u64,
    /// Number of Unicode characters.
    chars: usize,
}

/// Aggregated result for all requested files.
#[derive(Debug, Clone, Serialize)]
pub struct FileStatsResult {
    /// Per-file statistics.
    files:       Vec<FileStat>,
    /// Sum of line counts across all files.
    total_lines: usize,
    /// Sum of byte sizes across all files.
    total_bytes: u64,
}

/// Get line count, byte size, and character count for files.
#[derive(ToolDef)]
#[tool(
    name = "file-stats",
    description = "Get line count, byte size, and character count for one or more files. \
                   Equivalent to wc -l. Use this to understand file sizes before reading."
)]
pub struct FileStatsTool;

impl FileStatsTool {
    /// Create a new `FileStatsTool` instance.
    pub fn new() -> Self { Self }
}

/// Compute file statistics for the given paths.
///
/// Extracted from the tool impl so tests can call it without constructing a
/// [`ToolContext`].
async fn compute_stats(paths: &[String]) -> anyhow::Result<FileStatsResult> {
    let mut files = Vec::with_capacity(paths.len());
    let mut total_lines = 0usize;
    let mut total_bytes = 0u64;

    for path_str in paths {
        let path = if std::path::Path::new(path_str).is_absolute() {
            std::path::PathBuf::from(path_str)
        } else {
            rara_paths::workspace_dir().join(path_str)
        };

        let content = tokio::fs::read_to_string(&path)
            .await
            .context(format!("failed to read {}", path.display()))?;

        let meta = tokio::fs::metadata(&path)
            .await
            .context(format!("failed to stat {}", path.display()))?;

        let lines = content.lines().count();
        let bytes = meta.len();
        let chars = content.chars().count();

        total_lines += lines;
        total_bytes += bytes;

        files.push(FileStat {
            path: path.display().to_string(),
            lines,
            bytes,
            chars,
        });
    }

    Ok(FileStatsResult {
        files,
        total_lines,
        total_bytes,
    })
}

#[async_trait]
impl ToolExecute for FileStatsTool {
    type Output = FileStatsResult;
    type Params = FileStatsParams;

    async fn run(
        &self,
        params: FileStatsParams,
        _context: &ToolContext,
    ) -> anyhow::Result<FileStatsResult> {
        compute_stats(&params.paths).await
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;

    #[tokio::test]
    async fn counts_lines_correctly() {
        let mut tmp = NamedTempFile::new().expect("create temp file");
        writeln!(tmp, "line 1").expect("write");
        writeln!(tmp, "line 2").expect("write");
        writeln!(tmp, "line 3").expect("write");

        let path = tmp.path().to_str().expect("path").to_owned();
        let result = compute_stats(&[path]).await.expect("compute_stats");

        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].lines, 3);
        assert_eq!(result.total_lines, 3);
        assert!(result.files[0].bytes > 0);
        assert!(result.files[0].chars > 0);
    }

    #[tokio::test]
    async fn multiple_files_aggregates() {
        let mut tmp1 = NamedTempFile::new().expect("create temp file");
        writeln!(tmp1, "a").expect("write");
        writeln!(tmp1, "b").expect("write");

        let mut tmp2 = NamedTempFile::new().expect("create temp file");
        writeln!(tmp2, "x").expect("write");
        writeln!(tmp2, "y").expect("write");
        writeln!(tmp2, "z").expect("write");

        let paths = vec![
            tmp1.path().to_str().expect("path").to_owned(),
            tmp2.path().to_str().expect("path").to_owned(),
        ];
        let result = compute_stats(&paths).await.expect("compute_stats");

        assert_eq!(result.files.len(), 2);
        assert_eq!(result.files[0].lines, 2);
        assert_eq!(result.files[1].lines, 3);
        assert_eq!(result.total_lines, 5);
        assert_eq!(
            result.total_bytes,
            result.files[0].bytes + result.files[1].bytes
        );
    }

    #[tokio::test]
    async fn empty_file_returns_zero() {
        let tmp = NamedTempFile::new().expect("create temp file");
        let path = tmp.path().to_str().expect("path").to_owned();
        let result = compute_stats(&[path]).await.expect("compute_stats");

        assert_eq!(result.files[0].lines, 0);
        assert_eq!(result.files[0].bytes, 0);
        assert_eq!(result.files[0].chars, 0);
    }

    #[tokio::test]
    async fn nonexistent_file_returns_error() {
        let result = compute_stats(&["/tmp/nonexistent_file_stats_test_12345".to_owned()]).await;
        assert!(result.is_err());
    }
}
