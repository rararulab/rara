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

//! `tape-anchor` tool — create a named checkpoint with summary and next_steps.

use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    memory::{HandoffState, TapeService},
    session::SessionIndex,
    tool::{ToolContext, ToolExecute},
};

/// Parameters for `tape-anchor`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TapeAnchorParams {
    /// Name for the checkpoint (e.g. "topic/immich-setup").
    name:       String,
    /// Summary of the conversation up to this point.
    summary:    Option<String>,
    /// What should happen next.
    next_steps: Option<String>,
    /// Optional additional JSON state to attach.
    state:      Option<serde_json::Value>,
}

/// Result of a `tape-anchor` invocation.
#[derive(Debug, Serialize)]
pub struct TapeAnchorResult {
    anchor_name:          String,
    entries_after_anchor: usize,
}

/// Create a named checkpoint with summary and next_steps.
#[derive(ToolDef)]
#[tool(
    name = "tape-anchor",
    description = "Create a named checkpoint with summary and next_steps."
)]
pub(crate) struct TapeAnchorTool {
    tape_service: TapeService,
    tape_name:    String,
    #[allow(dead_code)]
    sessions:     Arc<dyn SessionIndex>,
}

impl TapeAnchorTool {
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
impl ToolExecute for TapeAnchorTool {
    type Output = TapeAnchorResult;
    type Params = TapeAnchorParams;

    async fn run(
        &self,
        params: TapeAnchorParams,
        _context: &ToolContext,
    ) -> anyhow::Result<TapeAnchorResult> {
        let handoff_state = HandoffState {
            summary: params.summary,
            next_steps: params.next_steps,
            owner: Some("agent".into()),
            extra: params.state,
            ..Default::default()
        };

        let entries = self
            .tape_service
            .handoff(&self.tape_name, &params.name, handoff_state)
            .await
            .context("tape-anchor")?;
        Ok(TapeAnchorResult {
            anchor_name:          params.name,
            entries_after_anchor: entries.len(),
        })
    }
}
