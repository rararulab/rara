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

//! Batch file editing primitive.
//!
//! Applies multiple exact-string replacements across one or more files in a
//! single tool call, reducing LLM round-trips for bulk edits. Edits targeting
//! the same file are applied sequentially in the order given.

use std::collections::BTreeMap;

use async_trait::async_trait;
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A single edit operation within a batch.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SingleEdit {
    /// Absolute path to the file to edit.
    pub file_path:   String,
    /// The exact string to find.
    pub old_string:  String,
    /// The replacement string.
    pub new_string:  String,
    /// Replace all occurrences (default false).
    pub replace_all: Option<bool>,
}

/// Parameters for the multi-edit tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct MultiEditParams {
    /// List of edit operations to apply. Edits targeting the same file are
    /// applied sequentially in the order given.
    edits: Vec<SingleEdit>,
}

/// Result of a single edit within the batch.
#[derive(Debug, Clone, Serialize)]
pub struct SingleEditResult {
    file_path:    String,
    success:      bool,
    replacements: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    error:        Option<String>,
}

/// Batch edit result.
#[derive(Debug, Clone, Serialize)]
pub struct MultiEditResult {
    results:       Vec<SingleEditResult>,
    total_success: usize,
    total_failed:  usize,
}

/// Apply multiple exact-string replacements across files in one call.
#[derive(ToolDef)]
#[tool(
    name = "multi-edit",
    description = "Apply multiple exact-string replacements across one or more files in a single \
                   call. Edits targeting the same file are applied sequentially in order. Partial \
                   failures are reported per-edit without rolling back successful ones. Use this \
                   instead of repeated edit-file calls.",
    tier = "deferred"
)]
pub struct MultiEditTool;

impl MultiEditTool {
    pub fn new() -> Self { Self }
}

use rara_kernel::guard::path_scope::resolve_and_guard;

/// Apply edits for a single file, grouped and applied sequentially in-memory.
///
/// Reads the file once, applies all edits in order to the in-memory content,
/// then writes once. This avoids TOCTOU issues when multiple edits target the
/// same file.
async fn apply_grouped_edits(
    file_path: &std::path::Path,
    edits: &[(usize, &SingleEdit)],
) -> Vec<(usize, SingleEditResult)> {
    let display_path = file_path.display().to_string();

    let mut content = match tokio::fs::read_to_string(file_path).await {
        Ok(c) => c,
        Err(e) => {
            // All edits for this file fail with the same read error.
            return edits
                .iter()
                .map(|(idx, _)| {
                    (
                        *idx,
                        SingleEditResult {
                            file_path:    display_path.clone(),
                            success:      false,
                            replacements: 0,
                            error:        Some(format!("failed to read file: {e}")),
                        },
                    )
                })
                .collect();
        }
    };

    let mut results = Vec::with_capacity(edits.len());
    let mut any_modified = false;

    for (idx, edit) in edits {
        let replace_all = edit.replace_all.unwrap_or(false);
        let count = content.matches(&edit.old_string).count();

        if count == 0 {
            results.push((
                *idx,
                SingleEditResult {
                    file_path:    display_path.clone(),
                    success:      false,
                    replacements: 0,
                    error:        Some(
                        "old_string not found. Make sure the string matches exactly.".into(),
                    ),
                },
            ));
            continue;
        }

        if !replace_all && count > 1 {
            results.push((
                *idx,
                SingleEditResult {
                    file_path:    display_path.clone(),
                    success:      false,
                    replacements: 0,
                    error:        Some(format!(
                        "old_string found {count} times. Use replace_all=true to replace all \
                         occurrences, or provide a more specific old_string."
                    )),
                },
            ));
            continue;
        }

        content = if replace_all {
            content.replace(&edit.old_string, &edit.new_string)
        } else {
            content.replacen(&edit.old_string, &edit.new_string, 1)
        };
        any_modified = true;

        results.push((
            *idx,
            SingleEditResult {
                file_path:    display_path.clone(),
                success:      true,
                replacements: if replace_all { count } else { 1 },
                error:        None,
            },
        ));
    }

    // Write back only if at least one edit succeeded.
    if any_modified {
        if let Err(e) = tokio::fs::write(file_path, &content).await {
            // Mark all previously-successful edits as failed.
            for (_, result) in &mut results {
                if result.success {
                    result.success = false;
                    result.replacements = 0;
                    result.error = Some(format!("failed to write file: {e}"));
                }
            }
        }
    }

    results
}

#[async_trait]
impl ToolExecute for MultiEditTool {
    type Output = MultiEditResult;
    type Params = MultiEditParams;

    async fn run(
        &self,
        params: MultiEditParams,
        _context: &ToolContext,
    ) -> anyhow::Result<MultiEditResult> {
        // Phase 1: resolve and guard all paths upfront.
        let mut resolved: Vec<(usize, std::path::PathBuf)> = Vec::new();
        let mut guard_errors: Vec<(usize, SingleEditResult)> = Vec::new();

        for (idx, edit) in params.edits.iter().enumerate() {
            match resolve_and_guard(&edit.file_path) {
                Ok(path) => resolved.push((idx, path)),
                Err(msg) => guard_errors.push((
                    idx,
                    SingleEditResult {
                        file_path:    edit.file_path.clone(),
                        success:      false,
                        replacements: 0,
                        error:        Some(msg),
                    },
                )),
            }
        }

        // Phase 2: group edits by file path (preserving original order).
        let mut by_file: BTreeMap<std::path::PathBuf, Vec<(usize, &SingleEdit)>> = BTreeMap::new();
        for (idx, path) in &resolved {
            by_file
                .entry(path.clone())
                .or_default()
                .push((*idx, &params.edits[*idx]));
        }

        // Phase 3: apply edits per file.
        let mut all_results: Vec<(usize, SingleEditResult)> = guard_errors;
        for (file_path, edits) in &by_file {
            all_results.extend(apply_grouped_edits(file_path, edits).await);
        }

        // Phase 4: sort results back to original edit order.
        all_results.sort_by_key(|(idx, _)| *idx);
        let results: Vec<SingleEditResult> = all_results.into_iter().map(|(_, r)| r).collect();

        let total_success = results.iter().filter(|r| r.success).count();
        let total_failed = results.len() - total_success;

        Ok(MultiEditResult {
            results,
            total_success,
            total_failed,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;

    /// Helper: apply grouped edits to a single temp file.
    async fn apply_edits_to_file(
        file_path: &std::path::Path,
        edits: Vec<SingleEdit>,
    ) -> Vec<SingleEditResult> {
        let indexed: Vec<(usize, &SingleEdit)> = edits.iter().enumerate().collect();
        apply_grouped_edits(file_path, &indexed)
            .await
            .into_iter()
            .map(|(_, r)| r)
            .collect()
    }

    #[tokio::test]
    async fn multi_file_replacement_works() {
        let mut f1 = NamedTempFile::new().expect("create temp file");
        write!(f1, "hello world").expect("write");
        let mut f2 = NamedTempFile::new().expect("create temp file");
        write!(f2, "foo bar baz").expect("write");

        let r1 = apply_edits_to_file(
            f1.path(),
            vec![SingleEdit {
                file_path:   f1.path().display().to_string(),
                old_string:  "hello".into(),
                new_string:  "goodbye".into(),
                replace_all: None,
            }],
        )
        .await;
        let r2 = apply_edits_to_file(
            f2.path(),
            vec![SingleEdit {
                file_path:   f2.path().display().to_string(),
                old_string:  "bar".into(),
                new_string:  "qux".into(),
                replace_all: None,
            }],
        )
        .await;

        assert!(r1[0].success, "edit 1 failed: {:?}", r1[0].error);
        assert!(r2[0].success, "edit 2 failed: {:?}", r2[0].error);

        let c1 = std::fs::read_to_string(f1.path()).expect("read");
        assert_eq!(c1, "goodbye world");
        let c2 = std::fs::read_to_string(f2.path()).expect("read");
        assert_eq!(c2, "foo qux baz");
    }

    #[tokio::test]
    async fn partial_failure_on_nonexistent_file() {
        let results = apply_edits_to_file(
            std::path::Path::new("/tmp/nonexistent_rara_test_12345.txt"),
            vec![SingleEdit {
                file_path:   "/tmp/nonexistent_rara_test_12345.txt".into(),
                old_string:  "x".into(),
                new_string:  "y".into(),
                replace_all: None,
            }],
        )
        .await;

        assert!(!results[0].success);
        assert!(
            results[0]
                .error
                .as_ref()
                .unwrap()
                .contains("failed to read")
        );
    }

    #[tokio::test]
    async fn old_string_not_found_reports_error() {
        let mut f1 = NamedTempFile::new().expect("create temp file");
        write!(f1, "hello world").expect("write");

        let results = apply_edits_to_file(
            f1.path(),
            vec![SingleEdit {
                file_path:   f1.path().display().to_string(),
                old_string:  "nonexistent".into(),
                new_string:  "replacement".into(),
                replace_all: None,
            }],
        )
        .await;

        assert!(!results[0].success);
        assert!(
            results[0]
                .error
                .as_ref()
                .unwrap()
                .contains("old_string not found")
        );
    }

    #[tokio::test]
    async fn same_file_edits_applied_sequentially() {
        let mut f = NamedTempFile::new().expect("create temp file");
        write!(f, "aaa bbb ccc").expect("write");

        let results = apply_edits_to_file(
            f.path(),
            vec![
                SingleEdit {
                    file_path:   f.path().display().to_string(),
                    old_string:  "aaa".into(),
                    new_string:  "xxx".into(),
                    replace_all: None,
                },
                SingleEdit {
                    file_path:   f.path().display().to_string(),
                    old_string:  "bbb".into(),
                    new_string:  "yyy".into(),
                    replace_all: None,
                },
            ],
        )
        .await;

        assert!(results[0].success);
        assert!(results[1].success);
        let content = std::fs::read_to_string(f.path()).expect("read");
        assert_eq!(content, "xxx yyy ccc");
    }

    #[tokio::test]
    async fn path_outside_workspace_blocked() {
        // resolve_and_guard calls rara_paths::workspace_dir() which may not be
        // available in CI. Skip if workspace creation would fail.
        let workspace = std::panic::catch_unwind(rara_paths::workspace_dir);
        let Ok(ws) = workspace else { return };

        // Use a path that is guaranteed to be outside the workspace.
        let outside = if ws.starts_with("/tmp") {
            "/etc/passwd"
        } else {
            "/tmp/__outside__"
        };
        let err = resolve_and_guard(outside);
        assert!(err.is_err());
        assert!(err.unwrap_err().contains("outside workspace"));
    }
}
