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

//! `tape-entries` tool — read tape entries from current or named anchor.

use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    memory::{TapEntry, TapEntryKind, TapeService},
    session::SessionIndex,
    tool::{ToolContext, ToolExecute},
};

/// Parameters for `tape-entries`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TapeEntriesParams {
    /// Read entries after this named anchor instead of the most recent one.
    after_anchor: Option<String>,
    /// Filter entries by kind (message, tool_call, tool_result, event, system,
    /// anchor).
    kinds:        Option<Vec<String>>,
}

/// Result of a `tape-entries` invocation.
#[derive(Debug, Serialize)]
pub struct TapeEntriesResult {
    entries: Vec<TapEntry>,
    count:   usize,
}

/// Read tape entries from current or named anchor.
#[derive(ToolDef)]
#[tool(
    name = "tape-entries",
    description = "Read tape entries from current or named anchor.",
    tier = "deferred"
)]
pub(crate) struct TapeEntriesTool {
    tape_service: TapeService,
    tape_name:    String,
    #[allow(dead_code)]
    sessions:     Arc<dyn SessionIndex>,
}

impl TapeEntriesTool {
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
impl ToolExecute for TapeEntriesTool {
    type Output = TapeEntriesResult;
    type Params = TapeEntriesParams;

    async fn run(
        &self,
        params: TapeEntriesParams,
        _context: &ToolContext,
    ) -> anyhow::Result<TapeEntriesResult> {
        let kind_filters: Option<Vec<TapEntryKind>> = params.kinds.map(|ks| {
            ks.iter()
                .filter_map(|k| k.parse::<TapEntryKind>().ok())
                .collect()
        });
        let kind_refs = kind_filters.as_deref();

        let entries = if let Some(anchor) = params.after_anchor.as_deref() {
            self.tape_service
                .after_anchor(&self.tape_name, anchor, kind_refs)
                .await
        } else {
            self.tape_service
                .from_last_anchor(&self.tape_name, kind_refs)
                .await
        }
        .context("tape-entries")?;

        let count = entries.len();
        Ok(TapeEntriesResult { entries, count })
    }
}
