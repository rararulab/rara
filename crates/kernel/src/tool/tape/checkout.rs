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

//! `tape-checkout` tool — fork a new session from a named anchor.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    memory::{TapeService, set_fork_metadata},
    session::{SessionEntry, SessionIndex, SessionKey},
    tool::{ToolContext, ToolExecute},
};

/// Parameters for `tape-checkout`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TapeCheckoutParams {
    /// Anchor name to fork from.
    name: String,
}

/// Result of a `tape-checkout` invocation.
#[derive(Debug, Serialize)]
pub struct TapeCheckoutResult {
    status:      String,
    from_anchor: String,
    new_session: String,
    message:     String,
}

/// Fork a new session from a named anchor.
#[derive(ToolDef)]
#[tool(
    name = "tape-checkout",
    description = "Fork a new session from a named anchor.",
    tier = "deferred"
)]
pub(crate) struct TapeCheckoutTool {
    tape_service: TapeService,
    tape_name:    String,
    sessions:     Arc<dyn SessionIndex>,
}

impl TapeCheckoutTool {
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
impl ToolExecute for TapeCheckoutTool {
    type Output = TapeCheckoutResult;
    type Params = TapeCheckoutParams;

    async fn run(
        &self,
        params: TapeCheckoutParams,
        _context: &ToolContext,
    ) -> anyhow::Result<TapeCheckoutResult> {
        let anchor_name = &params.name;

        // 1. Create a new session with fork metadata.
        let new_key = SessionKey::new();
        let mut metadata = None;
        set_fork_metadata(&mut metadata, &self.tape_name, anchor_name);
        let now = Utc::now();
        let entry = SessionEntry {
            key: new_key.clone(),
            title: Some(format!("Fork from {anchor_name}")),
            model: None,
            model_provider: None,
            thinking_level: None,
            system_prompt: None,
            total_entries: 0,
            preview: None,
            last_token_usage: None,
            estimated_context_tokens: 0,
            entries_since_last_anchor: 0,
            anchors: Vec::new(),
            metadata,
            created_at: now,
            updated_at: now,
        };

        self.sessions
            .create_session(&entry)
            .await
            .map_err(|e| anyhow::anyhow!("failed to create fork session: {e}"))?;

        // 2. Copy tape entries up to the anchor into the new tape.
        let new_tape = new_key.to_string();
        if let Err(e) = self
            .tape_service
            .checkout_anchor(&self.tape_name, anchor_name, &new_tape)
            .await
        {
            // Rollback session on tape failure.
            let _ = self.sessions.delete_session(&new_key).await;
            return Err(anyhow::anyhow!("checkout failed: {e}"));
        }

        Ok(TapeCheckoutResult {
            status:      "checked_out".into(),
            from_anchor: anchor_name.to_string(),
            new_session: new_tape.clone(),
            message:     format!(
                "Forked from anchor '{}'. New session: {}. Context has been reset to the anchor \
                 point.",
                anchor_name, new_tape
            ),
        })
    }
}
