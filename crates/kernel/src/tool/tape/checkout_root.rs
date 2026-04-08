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

//! `tape-checkout-root` tool — return to the root session from a fork.

use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use rara_tool_macro::ToolDef;
use serde::Serialize;

use crate::{
    memory::TapeService,
    session::SessionIndex,
    tool::{EmptyParams, ToolContext, ToolExecute},
};

/// Result of a `tape-checkout-root` invocation.
#[derive(Debug, Serialize)]
pub struct TapeCheckoutRootResult {
    status:          String,
    root_session:    Option<String>,
    current_session: Option<String>,
    message:         String,
}

/// Return to the root session from a forked session.
#[derive(ToolDef)]
#[tool(
    name = "tape-checkout-root",
    description = "Return to the root session from a forked session.",
    tier = "deferred",
    read_only,
    concurrency_safe
)]
pub(crate) struct TapeCheckoutRootTool {
    tape_service: TapeService,
    tape_name:    String,
    sessions:     Arc<dyn SessionIndex>,
}

impl TapeCheckoutRootTool {
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
impl ToolExecute for TapeCheckoutRootTool {
    type Output = TapeCheckoutRootResult;
    type Params = EmptyParams;

    async fn run(
        &self,
        _params: EmptyParams,
        _context: &ToolContext,
    ) -> anyhow::Result<TapeCheckoutRootResult> {
        let root = self
            .tape_service
            .find_root_session(&self.tape_name, self.sessions.as_ref())
            .await
            .context("tape-checkout-root")?;

        if root == self.tape_name {
            return Ok(TapeCheckoutRootResult {
                status:          "already_at_root".into(),
                root_session:    None,
                current_session: None,
                message:         "This session is already the root — there is no parent to return \
                                  to."
                .into(),
            });
        }

        Ok(TapeCheckoutRootResult {
            status:          "root_found".into(),
            root_session:    Some(root.clone()),
            current_session: Some(self.tape_name.clone()),
            message:         format!(
                "Root session is {}. Use this session ID to navigate back to the original \
                 conversation.",
                root
            ),
        })
    }
}
