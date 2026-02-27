use async_trait::async_trait;
use serde_json::Value;

use crate::guard::{Guard, GuardContext, Verdict};

/// A guard that allows everything — no approval or moderation.
pub struct NoopGuard;

#[async_trait]
impl Guard for NoopGuard {
    async fn check_tool(
        &self,
        _ctx: &GuardContext,
        _tool_name: &str,
        _args: &Value,
    ) -> Verdict {
        Verdict::Allow
    }

    async fn check_output(
        &self,
        _ctx: &GuardContext,
        _content: &str,
    ) -> Verdict {
        Verdict::Allow
    }
}
