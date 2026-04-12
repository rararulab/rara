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

//! Frecency-aware grep backed by fff-search.
//!
//! Wraps [`fff_search::grep::grep_search`] as an agent tool. Results from
//! frequently accessed files rank higher, and the search supports automatic
//! mode detection (plain text vs regex).

use std::fmt::Write as _;

use anyhow::Context as _;
use async_trait::async_trait;
use fff_query_parser::AiGrepConfig;
use fff_search::{
    QueryParser, SharedPicker,
    grep::{self, GrepMode, GrepSearchOptions, has_regex_metacharacters},
};
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const DEFAULT_LIMIT: usize = 30;

/// Input parameters for the fff-grep tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FffGrepParams {
    /// Search pattern. Plain text or regex — mode is auto-detected from
    /// metacharacters. Supports constraint prefixes (e.g. '*.rs pattern',
    /// 'src/ pattern').
    pattern: String,
    /// Maximum number of matching lines to return (default 30).
    limit:   Option<usize>,
}

/// Typed result returned by the fff-grep tool.
#[derive(Debug, Clone, Serialize)]
pub struct FffGrepResult {
    /// Formatted grep output with file paths, line numbers, and content.
    pub output:      String,
    /// Number of matches returned.
    pub match_count: usize,
}

/// Smart grep with frecency ranking.
///
/// Searches file contents with automatic mode detection (plain text
/// or regex based on metacharacters). Results from frequently
/// accessed files rank higher.
#[derive(ToolDef)]
#[tool(
    name = "fff-grep",
    description = "Smart grep with frecency ranking. Searches file contents with automatic mode \
                   detection (plain text or regex). Results from frequently accessed files rank \
                   higher. Supports constraint prefixes ('*.rs pattern', 'src/ pattern').",
    tier = "deferred",
    read_only,
    concurrency_safe
)]
pub struct FffGrepTool {
    picker: SharedPicker,
}

impl FffGrepTool {
    /// Create a new instance with shared fff state.
    pub fn new(picker: SharedPicker) -> Self { Self { picker } }
}

#[async_trait]
impl ToolExecute for FffGrepTool {
    type Output = FffGrepResult;
    type Params = FffGrepParams;

    async fn run(
        &self,
        params: FffGrepParams,
        _context: &ToolContext,
    ) -> anyhow::Result<FffGrepResult> {
        let limit = params.limit.unwrap_or(DEFAULT_LIMIT);
        let pattern = params.pattern.clone();
        let picker = self.picker.clone();

        tokio::task::spawn_blocking(move || do_fff_grep(&pattern, limit, &picker))
            .await
            .context("fff-grep task panicked")?
    }
}

/// Perform a frecency-aware grep search.
fn do_fff_grep(
    pattern: &str,
    limit: usize,
    shared_picker: &SharedPicker,
) -> anyhow::Result<FffGrepResult> {
    let guard = shared_picker
        .read()
        .map_err(|e| anyhow::anyhow!("failed to acquire picker lock: {e}"))?;
    let picker = guard
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("fff file picker not initialized"))?;

    let parser = QueryParser::new(AiGrepConfig);
    let parsed = parser.parse(pattern);
    let grep_text = parsed.grep_text();

    let mode = if has_regex_metacharacters(&grep_text) {
        GrepMode::Regex
    } else {
        GrepMode::PlainText
    };

    let options = GrepSearchOptions {
        max_file_size: 10 * 1024 * 1024,
        max_matches_per_file: 10,
        smart_case: true,
        file_offset: 0,
        page_limit: 50,
        mode,
        time_budget_ms: 0,
        before_context: 0,
        after_context: 3,
        classify_definitions: true,
        trim_whitespace: true,
    };

    let files = picker.get_files();
    let budget = picker.cache_budget();
    let result = grep::grep_search(files, &parsed, &options, budget, None, None, None);

    let mut output = String::new();
    let mut match_count = 0;

    if result.matches.is_empty() {
        output.push_str("0 matches.");
    } else {
        let mut current_file = "";
        for m in &result.matches {
            if match_count >= limit {
                let _ = writeln!(
                    output,
                    "... [{} more matches]",
                    result.matches.len() - limit
                );
                break;
            }

            let file = result.files[m.file_index];
            let path = file.relative_path();
            if path != current_file {
                if !current_file.is_empty() {
                    output.push('\n');
                }
                let _ = writeln!(output, "{}", path);
                current_file = path;
            }

            let _ = writeln!(output, "  {}:{}", m.line_number, m.line_content);
            match_count += 1;
        }
    }

    Ok(FffGrepResult {
        output: output.trim_end().to_owned(),
        match_count,
    })
}
