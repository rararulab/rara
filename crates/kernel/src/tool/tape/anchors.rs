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

//! `tape-anchors` tool — list recent tape anchors.

use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    memory::{AnchorSummary, TapeService},
    session::SessionIndex,
    tool::{ToolContext, ToolExecute},
};

/// Parameters for `tape-anchors`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TapeAnchorsParams {
    /// Maximum number of anchors to return (default: 10).
    limit: Option<usize>,
}

/// Result of a `tape-anchors` invocation.
#[derive(Debug, Serialize)]
pub struct TapeAnchorsResult {
    anchors: Vec<AnchorSummary>,
    count:   usize,
}

/// List recent tape anchors.
#[derive(ToolDef)]
#[tool(name = "tape-anchors", description = "List recent tape anchors.")]
pub(crate) struct TapeAnchorsTool {
    tape_service: TapeService,
    tape_name:    String,
    #[allow(dead_code)]
    sessions:     Arc<dyn SessionIndex>,
}

impl TapeAnchorsTool {
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
impl ToolExecute for TapeAnchorsTool {
    type Output = TapeAnchorsResult;
    type Params = TapeAnchorsParams;

    async fn run(
        &self,
        params: TapeAnchorsParams,
        _context: &ToolContext,
    ) -> anyhow::Result<TapeAnchorsResult> {
        let anchors = self
            .tape_service
            .anchors(&self.tape_name, params.limit.unwrap_or(10))
            .await
            .context("tape-anchors")?;
        let count = anchors.len();
        Ok(TapeAnchorsResult { anchors, count })
    }
}
