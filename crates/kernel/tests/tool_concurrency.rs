//! Integration tests: concurrency partitioning and safety axes.

use std::sync::Arc;

use async_trait::async_trait;
use rara_kernel::tool::{AgentTool, ToolCallBatch, ToolContext, ToolOutput, partition_tool_calls};

// ---------------------------------------------------------------------------
// Test tools with different safety profiles
// ---------------------------------------------------------------------------

struct ReadOnlyTool;

#[async_trait]
impl AgentTool for ReadOnlyTool {
    fn name(&self) -> &str { "read_file" }

    fn description(&self) -> &str { "reads a file" }

    fn parameters_schema(&self) -> serde_json::Value { serde_json::json!({}) }

    fn is_read_only(&self, _args: &serde_json::Value) -> bool { true }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool { true }

    async fn execute(&self, _p: serde_json::Value, _c: &ToolContext) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::from(serde_json::json!({})))
    }
}

struct WriteTool;

#[async_trait]
impl AgentTool for WriteTool {
    fn name(&self) -> &str { "write_file" }

    fn description(&self) -> &str { "writes a file" }

    fn parameters_schema(&self) -> serde_json::Value { serde_json::json!({}) }

    fn is_destructive(&self, _args: &serde_json::Value) -> bool { true }

    async fn execute(&self, _p: serde_json::Value, _c: &ToolContext) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::from(serde_json::json!({})))
    }
}

struct AskTool;

#[async_trait]
impl AgentTool for AskTool {
    fn name(&self) -> &str { "ask_user" }

    fn description(&self) -> &str { "asks user" }

    fn parameters_schema(&self) -> serde_json::Value { serde_json::json!({}) }

    fn requires_user_interaction(&self) -> bool { true }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool { true }

    async fn execute(&self, _p: serde_json::Value, _c: &ToolContext) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::from(serde_json::json!({})))
    }
}

// ---------------------------------------------------------------------------
// Test helper: a call struct that owns both tool and args.
// ---------------------------------------------------------------------------

struct FakeCall {
    tool: Arc<dyn AgentTool>,
    args: serde_json::Value,
}

impl FakeCall {
    fn new(tool: Arc<dyn AgentTool>) -> Self {
        Self {
            tool,
            args: serde_json::json!({}),
        }
    }
}

fn resolve(call: &FakeCall) -> Option<(&dyn AgentTool, &serde_json::Value)> {
    Some((call.tool.as_ref(), &call.args))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Default safety axes are fail-closed.
#[test]
fn default_axes_are_fail_closed() {
    let tool: Arc<dyn AgentTool> = Arc::new(WriteTool);
    let args = serde_json::json!({});
    assert!(!tool.is_concurrency_safe(&args));
    assert!(!tool.is_read_only(&args));
    assert!(!tool.requires_user_interaction());
}

/// Read-only tool reports correct axes.
#[test]
fn read_only_axes() {
    let tool: Arc<dyn AgentTool> = Arc::new(ReadOnlyTool);
    let args = serde_json::json!({});
    assert!(tool.is_read_only(&args));
    assert!(tool.is_concurrency_safe(&args));
    assert!(!tool.is_destructive(&args));
}

/// [Read, Read, Write, Read] → [Concurrent(2), Sequential(1), Concurrent(1)].
#[test]
fn partition_read_write_read() {
    let read: Arc<dyn AgentTool> = Arc::new(ReadOnlyTool);
    let write: Arc<dyn AgentTool> = Arc::new(WriteTool);

    let calls = vec![
        FakeCall::new(Arc::clone(&read)),
        FakeCall::new(Arc::clone(&read)),
        FakeCall::new(Arc::clone(&write)),
        FakeCall::new(Arc::clone(&read)),
    ];

    let batches = partition_tool_calls(calls, resolve);

    assert_eq!(batches.len(), 3);
    assert!(matches!(&batches[0], ToolCallBatch::Concurrent(v) if v.len() == 2));
    assert!(matches!(&batches[1], ToolCallBatch::Sequential(_)));
    assert!(matches!(&batches[2], ToolCallBatch::Concurrent(v) if v.len() == 1));
}

/// All safe tools → single Concurrent batch.
#[test]
fn all_safe_single_batch() {
    let read: Arc<dyn AgentTool> = Arc::new(ReadOnlyTool);
    let calls = vec![
        FakeCall::new(Arc::clone(&read)),
        FakeCall::new(Arc::clone(&read)),
        FakeCall::new(Arc::clone(&read)),
    ];

    let batches = partition_tool_calls(calls, resolve);

    assert_eq!(batches.len(), 1);
    assert!(matches!(&batches[0], ToolCallBatch::Concurrent(v) if v.len() == 3));
}

/// All unsafe tools → each gets its own Sequential batch.
#[test]
fn all_unsafe_sequential() {
    let write: Arc<dyn AgentTool> = Arc::new(WriteTool);
    let calls = vec![
        FakeCall::new(Arc::clone(&write)),
        FakeCall::new(Arc::clone(&write)),
    ];

    let batches = partition_tool_calls(calls, resolve);

    assert_eq!(batches.len(), 2);
    assert!(matches!(&batches[0], ToolCallBatch::Sequential(_)));
    assert!(matches!(&batches[1], ToolCallBatch::Sequential(_)));
}

/// Tool requiring user interaction is always sequential even if
/// concurrency_safe.
#[test]
fn user_interaction_forces_sequential() {
    let ask: Arc<dyn AgentTool> = Arc::new(AskTool);
    let read: Arc<dyn AgentTool> = Arc::new(ReadOnlyTool);
    let calls = vec![
        FakeCall::new(Arc::clone(&read)),
        FakeCall::new(Arc::clone(&ask)),
        FakeCall::new(Arc::clone(&read)),
    ];

    let batches = partition_tool_calls(calls, resolve);

    assert_eq!(batches.len(), 3);
    assert!(matches!(&batches[0], ToolCallBatch::Concurrent(v) if v.len() == 1));
    assert!(matches!(&batches[1], ToolCallBatch::Sequential(_)));
    assert!(matches!(&batches[2], ToolCallBatch::Concurrent(v) if v.len() == 1));
}

/// Empty input → empty output.
#[test]
fn empty_calls() {
    let batches = partition_tool_calls(Vec::<FakeCall>::new(), resolve);
    assert!(batches.is_empty());
}
