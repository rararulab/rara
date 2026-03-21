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

//! Batch file editing primitive.
//!
//! Applies multiple exact-string replacements across one or more files in a
//! single tool call, reducing LLM round-trips for bulk edits.

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
    /// List of edit operations to apply. Each is an independent replacement.
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
                   call. Each edit is independent — partial failures are reported per-edit \
                   without rolling back successful ones. Use this instead of repeated edit-file \
                   calls."
)]
pub struct MultiEditTool;

impl MultiEditTool {
    pub fn new() -> Self { Self }
}

/// Apply a single edit operation and return the result.
async fn apply_single_edit(edit: &SingleEdit) -> SingleEditResult {
    let file_path = if std::path::Path::new(&edit.file_path).is_absolute() {
        std::path::PathBuf::from(&edit.file_path)
    } else {
        rara_paths::workspace_dir().join(&edit.file_path)
    };
    let display_path = file_path.display().to_string();

    let content = match tokio::fs::read_to_string(&file_path).await {
        Ok(c) => c,
        Err(e) => {
            return SingleEditResult {
                file_path:    display_path,
                success:      false,
                replacements: 0,
                error:        Some(format!("failed to read file: {e}")),
            };
        }
    };

    let replace_all = edit.replace_all.unwrap_or(false);
    let count = content.matches(&edit.old_string).count();

    if count == 0 {
        return SingleEditResult {
            file_path:    display_path,
            success:      false,
            replacements: 0,
            error:        Some(
                "old_string not found. Make sure the string matches exactly.".into(),
            ),
        };
    }

    if !replace_all && count > 1 {
        return SingleEditResult {
            file_path:    display_path,
            success:      false,
            replacements: 0,
            error:        Some(format!(
                "old_string found {count} times. Use replace_all=true to replace all occurrences, \
                 or provide a more specific old_string."
            )),
        };
    }

    let new_content = if replace_all {
        content.replace(&edit.old_string, &edit.new_string)
    } else {
        content.replacen(&edit.old_string, &edit.new_string, 1)
    };

    if let Err(e) = tokio::fs::write(&file_path, &new_content).await {
        return SingleEditResult {
            file_path:    display_path,
            success:      false,
            replacements: 0,
            error:        Some(format!("failed to write file: {e}")),
        };
    }

    SingleEditResult {
        file_path:    display_path,
        success:      true,
        replacements: if replace_all { count } else { 1 },
        error:        None,
    }
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
        let mut results = Vec::with_capacity(params.edits.len());
        for edit in &params.edits {
            results.push(apply_single_edit(edit).await);
        }

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

    #[tokio::test]
    async fn multi_file_replacement_works() {
        let mut f1 = NamedTempFile::new().expect("create temp file");
        write!(f1, "hello world").expect("write");
        let mut f2 = NamedTempFile::new().expect("create temp file");
        write!(f2, "foo bar baz").expect("write");

        let edits = [
            SingleEdit {
                file_path:   f1.path().display().to_string(),
                old_string:  "hello".into(),
                new_string:  "goodbye".into(),
                replace_all: None,
            },
            SingleEdit {
                file_path:   f2.path().display().to_string(),
                old_string:  "bar".into(),
                new_string:  "qux".into(),
                replace_all: None,
            },
        ];

        let r1 = apply_single_edit(&edits[0]).await;
        let r2 = apply_single_edit(&edits[1]).await;
        assert!(r1.success, "edit 1 failed: {:?}", r1.error);
        assert!(r2.success, "edit 2 failed: {:?}", r2.error);
        assert_eq!(r1.replacements, 1);
        assert_eq!(r2.replacements, 1);

        let c1 = std::fs::read_to_string(f1.path()).expect("read");
        assert_eq!(c1, "goodbye world");
        let c2 = std::fs::read_to_string(f2.path()).expect("read");
        assert_eq!(c2, "foo qux baz");
    }

    #[tokio::test]
    async fn partial_failure_on_nonexistent_file() {
        let mut f1 = NamedTempFile::new().expect("create temp file");
        write!(f1, "hello world").expect("write");

        let good = SingleEdit {
            file_path:   f1.path().display().to_string(),
            old_string:  "hello".into(),
            new_string:  "goodbye".into(),
            replace_all: None,
        };
        let bad = SingleEdit {
            file_path:   "/tmp/nonexistent_rara_test_file_12345.txt".into(),
            old_string:  "x".into(),
            new_string:  "y".into(),
            replace_all: None,
        };

        let r_good = apply_single_edit(&good).await;
        let r_bad = apply_single_edit(&bad).await;

        assert!(r_good.success);
        assert!(!r_bad.success);
        assert!(r_bad.error.as_ref().unwrap().contains("failed to read"));
    }

    #[tokio::test]
    async fn old_string_not_found_reports_error() {
        let mut f1 = NamedTempFile::new().expect("create temp file");
        write!(f1, "hello world").expect("write");

        let edit = SingleEdit {
            file_path:   f1.path().display().to_string(),
            old_string:  "nonexistent".into(),
            new_string:  "replacement".into(),
            replace_all: None,
        };

        let result = apply_single_edit(&edit).await;
        assert!(!result.success);
        assert!(
            result
                .error
                .as_ref()
                .unwrap()
                .contains("old_string not found")
        );
    }
}
