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

//! `tape-search` tool — keyword search across the entire tape.

use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    memory::{TapEntry, TapeService},
    session::SessionIndex,
    tool::{ToolContext, ToolExecute},
};

/// Parameters for `tape-search`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TapeSearchParams {
    /// Text to search for in past conversations.
    query: String,
    /// Maximum number of results (default: 10).
    limit: Option<usize>,
}

/// Result of a `tape-search` invocation.
#[derive(Debug, Serialize)]
pub struct TapeSearchResult {
    results: Vec<TapEntry>,
    count:   usize,
}

/// Search past conversations by keyword across all anchors.
#[derive(ToolDef)]
#[tool(
    name = "tape-search",
    description = "Search past conversations by keyword across all anchors.",
    read_only,
    concurrency_safe
)]
pub(crate) struct TapeSearchTool {
    tape_service: TapeService,
    tape_name:    String,
    #[allow(dead_code)]
    sessions:     Arc<dyn SessionIndex>,
}

impl TapeSearchTool {
    pub fn new(
        tape_service: TapeService,
        tape_name: String,
        sessions: Arc<dyn SessionIndex>,
    ) -> Self {
        Self {
            tape_service,
            tape_name,
            sessions,
        }
    }
}

#[async_trait]
impl ToolExecute for TapeSearchTool {
    type Output = TapeSearchResult;
    type Params = TapeSearchParams;

    async fn run(
        &self,
        params: TapeSearchParams,
        _context: &ToolContext,
    ) -> anyhow::Result<TapeSearchResult> {
        let results = self
            .tape_service
            .search(
                &self.tape_name,
                &params.query,
                params.limit.unwrap_or(10),
                false,
            )
            .await
            .context("tape-search")?;
        let count = results.len();
        Ok(TapeSearchResult { results, count })
    }
}
