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

//! Ask-user tool — blocks the agent until the user responds.
//!
//! Uses [`rara_kernel::user_question::UserQuestionManager`] to submit a
//! question and wait for the user's answer via the same oneshot-channel pattern
//! as `ApprovalManager`.

use std::time::Duration;

use async_trait::async_trait;
use rara_kernel::{
    tool::{ToolContext, ToolExecute},
    user_question::UserQuestionManagerRef,
};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

/// Default timeout for user questions (5 minutes).
const DEFAULT_TIMEOUT_SECS: u64 = 300;

/// Parameters for the ask-user tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AskUserParams {
    /// The question to ask the user. Be specific about what information you
    /// need and why.
    question: String,
}

/// Ask the user a question and wait for their response.
#[derive(ToolDef)]
#[tool(
    name = "ask-user",
    description = "Ask the user a question and wait for their response. Use when you need \
                   information that only the user can provide (e.g. API keys, preferences, \
                   clarifications). The agent will pause until the user responds or the request \
                   times out.",
    tier = "deferred",
    user_interaction
)]
pub struct AskUserTool {
    manager: UserQuestionManagerRef,
}

impl AskUserTool {
    /// Create a new ask-user tool backed by the given question manager.
    pub fn new(manager: UserQuestionManagerRef) -> Self { Self { manager } }
}

#[async_trait]
impl ToolExecute for AskUserTool {
    type Output = Value;
    type Params = AskUserParams;

    #[tracing::instrument(skip_all)]
    async fn run(&self, params: AskUserParams, context: &ToolContext) -> anyhow::Result<Value> {
        let timeout = Duration::from_secs(DEFAULT_TIMEOUT_SECS);
        // Propagate the originating endpoint so channel adapters can route the
        // question back to the same conversation surface (e.g. a Telegram
        // forum topic) instead of a default fallback like `primary_chat_id`.
        let endpoint = context.origin_endpoint.clone();
        let answer = self.manager.ask(params.question, endpoint, timeout).await?;
        Ok(serde_json::json!({ "answer": answer }))
    }
}
