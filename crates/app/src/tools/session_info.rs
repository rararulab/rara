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

//! Session info tool for querying current session metadata.

use async_trait::async_trait;
use rara_kernel::{
    session::SessionIndexRef,
    tool::{ToolContext, ToolExecute},
};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SessionInfoParams {}

#[derive(Debug, Clone, Serialize)]
pub struct SessionInfoResult {
    pub session_key: String,
    pub title:       Option<String>,
    pub metadata:    Option<Value>,
}

/// Agent tool that retrieves metadata for the current session.
#[derive(ToolDef)]
#[tool(
    name = "get-session-info",
    description = "Get metadata for the current session, including uploaded image paths and other \
                   session-specific information.",
    bypass_interceptor,
    tier = "deferred"
)]
pub struct SessionInfoTool {
    session_index: SessionIndexRef,
}
impl SessionInfoTool {
    pub fn new(session_index: SessionIndexRef) -> Self { Self { session_index } }
}

#[async_trait]
impl ToolExecute for SessionInfoTool {
    type Output = SessionInfoResult;
    type Params = SessionInfoParams;

    async fn run(
        &self,
        _params: SessionInfoParams,
        context: &ToolContext,
    ) -> anyhow::Result<SessionInfoResult> {
        let session_key = &context.session_key;
        let entry = self
            .session_index
            .get_session(session_key)
            .await
            .map_err(|e| anyhow::anyhow!("failed to get session: {e}"))?
            .ok_or_else(|| anyhow::anyhow!("session not found"))?;
        Ok(SessionInfoResult {
            session_key: entry.key.to_string(),
            title:       entry.title,
            metadata:    entry.metadata,
        })
    }
}
