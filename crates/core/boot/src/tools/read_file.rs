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

use anyhow::Context;
use async_trait::async_trait;
use rara_kernel::tool::AgentTool;
use serde_json::json;

/// Maximum total output size in bytes (50 KB).
const MAX_OUTPUT_BYTES: usize = 50 * 1024;

/// Maximum characters per line before truncation.
const MAX_LINE_CHARS: usize = 2000;

/// Default maximum number of lines to return.
const DEFAULT_LIMIT: usize = 2000;

/// Number of bytes to check for binary detection.
const BINARY_CHECK_BYTES: usize = 1024;

/// Layer 1 primitive: read a file with line numbers.
pub struct ReadFileTool;

impl ReadFileTool {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl AgentTool for ReadFileTool {
    fn name(&self) -> &str { "read_file" }

    fn description(&self) -> &str {
        "Read a file from the filesystem. Returns content with line number prefixes (like cat -n). \
         Supports offset and limit for paginated reading. Detects binary files. Long lines are \
         truncated at 2000 characters."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file to read"
                },
                "offset": {
                    "type": "number",
                    "description": "1-based line number to start reading from (default 1)"
                },
                "limit": {
                    "type": "number",
                    "description": "Maximum number of lines to return (default 2000)"
                }
            },
            "required": ["file_path"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let file_path = params
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: file_path"))?;

        let offset = params
            .get("offset")
            .and_then(|v| v.as_u64())
            .map(|v| v.max(1) as usize)
            .unwrap_or(1);

        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(DEFAULT_LIMIT);

        let raw_bytes = tokio::fs::read(file_path)
            .await
            .context(format!("failed to read file {file_path}"))?;

        // Binary detection: check for null bytes in the first BINARY_CHECK_BYTES.
        let check_len = raw_bytes.len().min(BINARY_CHECK_BYTES);
        if raw_bytes[..check_len].contains(&0) {
            return Ok(json!({
                "content": "[binary file detected]",
                "total_lines": 0,
                "truncated": false,
            }));
        }

        let content = String::from_utf8_lossy(&raw_bytes);
        let all_lines: Vec<&str> = content.lines().collect();
        let total_lines = all_lines.len();

        // Apply offset (1-based) and limit.
        let start_idx = (offset - 1).min(total_lines);
        let end_idx = (start_idx + limit).min(total_lines);
        let selected = &all_lines[start_idx..end_idx];

        let mut output = String::new();
        let mut truncated = false;

        for (i, line) in selected.iter().enumerate() {
            let line_no = start_idx + i + 1;
            let display_line = if line.len() > MAX_LINE_CHARS {
                truncated = true;
                format!("{}... [truncated]", &line[..MAX_LINE_CHARS])
            } else {
                (*line).to_owned()
            };

            let formatted = format!("{line_no:>6}\t{display_line}\n");

            if output.len() + formatted.len() > MAX_OUTPUT_BYTES {
                truncated = true;
                break;
            }
            output.push_str(&formatted);
        }

        if end_idx < total_lines {
            truncated = true;
        }

        Ok(json!({
            "content": output,
            "total_lines": total_lines,
            "truncated": truncated,
        }))
    }
}
