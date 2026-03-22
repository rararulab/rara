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

//! `tape-info` tool — returns tape state metadata.

use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    memory::TapeService,
    session::SessionIndex,
    tool::{ToolContext, ToolExecute},
};

/// Result of a `tape-info` invocation.
#[derive(Debug, Serialize)]
pub struct TapeInfoResult {
    tape_name:                 String,
    total_entries:             usize,
    anchor_count:              usize,
    last_anchor:               Option<String>,
    entries_since_last_anchor: usize,
    last_token_usage:          Option<u64>,
    estimated_context_tokens:  u64,
}

/// Parameters for `tape-info` (none required).
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TapeInfoParams {}

/// Show tape state: entry count, anchor count, context token estimate.
#[derive(ToolDef)]
#[tool(
    name = "tape-info",
    description = "Show tape state: entry count, anchor count, context token estimate.",
    tier = "deferred"
)]
pub(crate) struct TapeInfoTool {
    tape_service: TapeService,
    tape_name:    String,
    #[allow(dead_code)]
    sessions:     Arc<dyn SessionIndex>,
}

impl TapeInfoTool {
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
impl ToolExecute for TapeInfoTool {
    type Output = TapeInfoResult;
    type Params = TapeInfoParams;

    async fn run(
        &self,
        _params: TapeInfoParams,
        _context: &ToolContext,
    ) -> anyhow::Result<TapeInfoResult> {
        let info = self
            .tape_service
            .info(&self.tape_name)
            .await
            .context("tape-info")?;
        Ok(TapeInfoResult {
            tape_name:                 info.name,
            total_entries:             info.entries,
            anchor_count:              info.anchors,
            last_anchor:               info.last_anchor,
            entries_since_last_anchor: info.entries_since_last_anchor,
            last_token_usage:          info.last_token_usage,
            estimated_context_tokens:  info.estimated_context_tokens,
        })
    }
}
