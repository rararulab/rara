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

use async_trait::async_trait;
use serde_json::Value;

use crate::guard::{Guard, GuardContext, Verdict};

/// A guard that allows everything — no approval or moderation.
pub struct NoopGuard;

#[async_trait]
impl Guard for NoopGuard {
    async fn check_tool(&self, _ctx: &GuardContext, _tool_name: &str, _args: &Value) -> Verdict {
        Verdict::Allow
    }

    async fn check_output(&self, _ctx: &GuardContext, _content: &str) -> Verdict { Verdict::Allow }
}
