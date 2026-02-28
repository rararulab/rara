// Copyright 2025 Crrow
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

//! Guard abstraction — tool approval and output moderation.

use async_trait::async_trait;
use serde_json::Value;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Context provided to guard checks.
#[derive(Debug, Clone)]
pub struct GuardContext {
    pub agent_id:   Uuid,
    pub user_id:    Uuid,
    pub session_id: Uuid,
}

/// Result of a guard check.
#[derive(Debug, Clone)]
pub enum Verdict {
    /// Action is allowed.
    Allow,
    /// Action is denied.
    Deny { reason: String },
    /// Action needs human approval before proceeding.
    NeedApproval { prompt: String },
}

impl Verdict {
    /// Returns `true` if the verdict allows the action.
    pub fn is_allow(&self) -> bool { matches!(self, Verdict::Allow) }

    /// Returns `true` if the verdict denies the action.
    pub fn is_deny(&self) -> bool { matches!(self, Verdict::Deny { .. }) }
}

// ---------------------------------------------------------------------------
// Guard trait
// ---------------------------------------------------------------------------

/// Intercepts tool execution and output for approval/moderation.
#[async_trait]
pub trait Guard: Send + Sync {
    /// Check whether a tool call should be allowed.
    async fn check_tool(&self, ctx: &GuardContext, tool_name: &str, args: &Value) -> Verdict;

    /// Check whether model output should be allowed (content moderation).
    async fn check_output(&self, ctx: &GuardContext, content: &str) -> Verdict;
}
