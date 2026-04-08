//! Integration tests: AgentTool::validate semantics.

use std::sync::Arc;

use async_trait::async_trait;
use rara_kernel::tool::{AgentTool, ToolContext, ToolOutput};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Tool that always passes validation (default trait impl).
struct PassTool;

#[async_trait]
impl AgentTool for PassTool {
    fn name(&self) -> &str { "pass" }

    fn description(&self) -> &str { "always passes" }

    fn parameters_schema(&self) -> serde_json::Value { serde_json::json!({}) }

    async fn execute(
        &self,
        _params: serde_json::Value,
        _ctx: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::from(serde_json::json!({ "ok": true })))
    }
}

/// Tool that rejects inputs where `"reject"` is `true`.
struct RejectTool;

#[async_trait]
impl AgentTool for RejectTool {
    fn name(&self) -> &str { "reject" }

    fn description(&self) -> &str { "rejects when reject=true" }

    fn parameters_schema(&self) -> serde_json::Value { serde_json::json!({}) }

    async fn validate(&self, params: &serde_json::Value) -> anyhow::Result<()> {
        if params
            .get("reject")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            anyhow::bail!("rejected by validate");
        }
        Ok(())
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
        _ctx: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::from(serde_json::json!({ "executed": true })))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Default validate implementation passes any input.
#[tokio::test]
async fn default_validate_passes() {
    let tool: Arc<dyn AgentTool> = Arc::new(PassTool);
    let result = tool.validate(&serde_json::json!({ "anything": 42 })).await;
    assert!(result.is_ok());
}

/// Custom validate rejects matching input.
#[tokio::test]
async fn custom_validate_rejects() {
    let tool: Arc<dyn AgentTool> = Arc::new(RejectTool);

    let ok = tool.validate(&serde_json::json!({ "reject": false })).await;
    assert!(ok.is_ok());

    let err = tool.validate(&serde_json::json!({ "reject": true })).await;
    assert!(err.is_err());
    assert!(
        err.unwrap_err()
            .to_string()
            .contains("rejected by validate"),
        "error message should mention the rejection reason"
    );
}

/// Validate is independent of execute — a tool can pass validate and then
/// execute, or fail validate without execute ever being called.
#[tokio::test]
async fn validate_is_independent_of_execute() {
    let tool: Arc<dyn AgentTool> = Arc::new(RejectTool);

    // Validate fails → caller should NOT call execute.
    let v = tool.validate(&serde_json::json!({ "reject": true })).await;
    assert!(v.is_err());

    // Validate passes → caller proceeds to execute.
    let v = tool.validate(&serde_json::json!({ "reject": false })).await;
    assert!(v.is_ok());
}
