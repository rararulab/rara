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

//! Frecency-aware fuzzy file finder backed by fff-search.
//!
//! Wraps [`fff_search::file_picker::FilePicker::fuzzy_search`] as an agent
//! tool. Results are ranked by a combination of fuzzy match score and
//! frecency (frequency + recency of access), so frequently used files
//! surface faster over time.

use std::fmt::Write as _;

use anyhow::Context as _;
use async_trait::async_trait;
use fff_search::{
    FuzzySearchOptions, QueryParser, SharedPicker, SharedQueryTracker, file_picker::FilePicker,
    types::PaginationArgs,
};
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const DEFAULT_LIMIT: usize = 20;

/// Input parameters for the fff-find tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FffFindParams {
    /// Fuzzy search query. Supports path prefixes (e.g. 'src/') and glob
    /// constraints. Keep queries short — prefer 1-2 terms.
    query: String,
    /// Maximum number of results to return (default 20).
    limit: Option<usize>,
}

/// Typed result returned by the fff-find tool.
#[derive(Debug, Clone, Serialize)]
pub struct FffFindResult {
    /// Matched file paths with scores.
    pub output:        String,
    /// Number of results returned.
    pub result_count:  usize,
    /// Total number of fuzzy matches (before limiting).
    pub total_matched: usize,
}

/// Frecency-aware fuzzy file search.
///
/// Results are ranked by how often and recently files were accessed,
/// combined with fuzzy match quality. Use for project-local searches
/// where access history helps find the right file faster.
#[derive(ToolDef)]
#[tool(
    name = "fff-find",
    description = "Frecency-aware fuzzy file search. Results ranked by access frequency/recency \
                   combined with fuzzy match quality. Supports path prefixes ('src/') and glob \
                   constraints. Keep queries short (1-2 terms).",
    tier = "deferred",
    read_only,
    concurrency_safe
)]
pub struct FffFindTool {
    picker:        SharedPicker,
    query_tracker: SharedQueryTracker,
}

impl FffFindTool {
    /// Create a new instance with shared fff state.
    pub fn new(picker: SharedPicker, query_tracker: SharedQueryTracker) -> Self {
        Self {
            picker,
            query_tracker,
        }
    }
}

#[async_trait]
impl ToolExecute for FffFindTool {
    type Output = FffFindResult;
    type Params = FffFindParams;

    async fn run(
        &self,
        params: FffFindParams,
        _context: &ToolContext,
    ) -> anyhow::Result<FffFindResult> {
        let limit = params.limit.unwrap_or(DEFAULT_LIMIT);
        let query_str = params.query.clone();
        let picker = self.picker.clone();
        let query_tracker = self.query_tracker.clone();

        tokio::task::spawn_blocking(move || do_fff_find(&query_str, limit, &picker, &query_tracker))
            .await
            .context("fff-find task panicked")?
    }
}

/// Perform a frecency-aware fuzzy file search.
fn do_fff_find(
    query: &str,
    limit: usize,
    shared_picker: &SharedPicker,
    shared_qt: &SharedQueryTracker,
) -> anyhow::Result<FffFindResult> {
    let guard = shared_picker
        .read()
        .map_err(|e| anyhow::anyhow!("failed to acquire picker lock: {e}"))?;
    let picker = guard
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("fff file picker not initialized"))?;

    let qt_guard = shared_qt
        .read()
        .map_err(|e| anyhow::anyhow!("failed to acquire query tracker lock: {e}"))?;

    let parser = QueryParser::default();
    let fff_query = parser.parse(query);

    let result = FilePicker::fuzzy_search(
        picker.get_files(),
        &fff_query,
        qt_guard.as_ref(),
        FuzzySearchOptions {
            max_threads:                  0,
            current_file:                 None,
            project_path:                 Some(picker.base_path()),
            combo_boost_score_multiplier: 100,
            min_combo_count:              3,
            pagination:                   PaginationArgs { offset: 0, limit },
        },
    );

    let total_matched = result.total_matched;
    let items = &result.items;
    let scores = &result.scores;

    let mut output = String::new();

    if items.is_empty() {
        let _ = write!(output, "0 results ({} indexed)", result.total_files);
    } else {
        if total_matched > items.len() {
            let _ = writeln!(output, "{}/{} matches", items.len(), total_matched);
        }

        for (i, item) in items.iter().enumerate() {
            let frecency = item.total_frecency_score();
            let score = scores.get(i).map(|s| s.total).unwrap_or(0);
            if frecency > 0 {
                let _ = writeln!(
                    output,
                    "{}  (score:{}, frecency:{})",
                    item.relative_path(),
                    score,
                    frecency
                );
            } else {
                let _ = writeln!(output, "{}  (score:{})", item.relative_path(), score);
            }
        }
    }

    Ok(FffFindResult {
        output: output.trim_end().to_owned(),
        result_count: items.len(),
        total_matched,
    })
}
