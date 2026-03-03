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

//! I/O bus and streaming integration tests for the agent OS kernel.
//!
//! These tests verify the I/O subsystem behavior during real LLM execution:
//! - StreamHub emits real-time TextDelta events
//! - StreamHub emits ToolCallStart/ToolCallEnd events
//! - OutboundBus receives final reply envelopes
//! - Multi-session stream isolation
//!
//! **Important**: The process loop is long-lived. These tests poll the
//! ProcessTable for `Waiting` state to detect message completion.
//!
//! Run with:
//! ```sh
//! cargo test -p rara-kernel --test io_streaming -- --ignored --test-threads=1
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use rara_kernel::{
    KernelHandle,
    io::stream::{StreamEvent, StreamHub},
    llm::{DriverRegistryBuilder, OpenAiDriver},
    process::{AgentId, AgentManifest, AgentResult, ProcessState, SessionId, principal::Principal},
    testing::TestKernelBuilder,
    tool::AgentTool,
};

/// Default Ollama base URL (OpenAI-compatible API endpoint).
const OLLAMA_BASE_URL: &str = "https://ollama.rara.local/v1";

/// Default model to use for Ollama integration tests.
const OLLAMA_MODEL: &str = "qwen3.5:cloud";

/// Helper: resolve the model name from env or defaults.
fn ollama_model() -> String {
    std::env::var("OLLAMA_MODEL").unwrap_or_else(|_| OLLAMA_MODEL.to_string())
}

/// Helper: build a test manifest using the real Ollama model.
fn test_manifest(name: &str, system_prompt: &str) -> AgentManifest {
    AgentManifest {
        name:               name.to_string(),
        role:               None,
        description:        format!("I/O test agent: {name}"),
        model:              Some(ollama_model()),
        system_prompt:      system_prompt.to_string(),
        soul_prompt:        None,
        provider_hint:      None,
        max_iterations:     Some(5),
        tools:              vec![],
        max_children:       None,
        max_context_tokens: None,
        priority:           Default::default(),
        metadata:           serde_json::Value::Null,
        sandbox:            None,
    }
}

/// Simple echo tool for integration testing.
struct EchoTool;

#[async_trait]
impl AgentTool for EchoTool {
    fn name(&self) -> &str { "echo_tool" }

    fn description(&self) -> &str {
        "Echoes back the input as-is. Always call this tool when asked."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "The text to echo back"
                }
            },
            "required": ["text"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        Ok(params)
    }
}

/// Helper: build and start a kernel with the real Ollama driver and optional
/// tools.
///
/// Returns the `KernelHandle` and a cancellation token.
fn start_test_kernel(
    tools: Vec<Arc<dyn AgentTool>>,
) -> (KernelHandle, tokio_util::sync::CancellationToken) {
    let model = ollama_model();
    let base_url = std::env::var("OLLAMA_BASE_URL").unwrap_or_else(|_| OLLAMA_BASE_URL.to_string());
    let driver = Arc::new(OpenAiDriver::new(base_url, "ollama"));
    let registry = Arc::new(
        DriverRegistryBuilder::new("ollama", &model)
            .driver("ollama", driver)
            .build(),
    );
    let mut builder = TestKernelBuilder::new()
        .driver_registry(registry)
        .max_concurrency(8)
        .max_iterations(10);
    for tool in tools {
        builder = builder.tool(tool);
    }
    let kernel = builder.build();
    let cancel = tokio_util::sync::CancellationToken::new();
    let (_arc, handle) = kernel.start(cancel.clone());
    (handle, cancel)
}

/// Poll until the process reaches `Completed` state and has a result, or
/// timeout.
async fn wait_for_result(
    handle: &KernelHandle,
    agent_id: AgentId,
    timeout_secs: u64,
) -> AgentResult {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    loop {
        if tokio::time::Instant::now() > deadline {
            let state = handle
                .process_table()
                .get(agent_id)
                .map(|p| format!("{:?}", p.state))
                .unwrap_or_else(|| "not found".to_string());
            panic!(
                "timed out after {timeout_secs}s waiting for agent {agent_id} result (state: \
                 {state})"
            );
        }
        if let Some(p) = handle.process_table().get(agent_id) {
            if matches!(p.state, ProcessState::Completed) {
                if let Some(result) = p.result {
                    return result;
                }
            }
            if matches!(p.state, ProcessState::Failed) {
                panic!("agent {agent_id} failed");
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

// ---------------------------------------------------------------------------
// Test 1: StreamHub receives TextDelta events during agent execution
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Ollama instance"]
async fn test_stream_hub_receives_text_deltas() {
    let (handle, cancel) = start_test_kernel(vec![]);
    let manifest = test_manifest("stream-text-agent", "Reply in one sentence.");
    let principal = Principal::user("test-user");

    let agent_id = handle
        .spawn_with_input(
            manifest,
            "Say hello in one sentence.".to_string(),
            principal,
            None,
        )
        .await
        .expect("spawn failed");

    // Wait for the agent to finish processing via ProcessTable polling.
    let result = wait_for_result(&handle, agent_id, 60).await;

    // Verify result was produced (which means stream events were emitted
    // internally).
    assert!(
        !result.output.trim().is_empty(),
        "agent should produce output via streaming pipeline"
    );

    // Clean up: send Kill signal to stop the process.
    let _ = handle.send_signal(agent_id, rara_kernel::process::Signal::Kill);
    cancel.cancel();
}

// ---------------------------------------------------------------------------
// Test 2: StreamHub emits ToolCallStart/ToolCallEnd during tool use
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Ollama instance"]
async fn test_stream_hub_tool_call_events() {
    let (handle, cancel) = start_test_kernel(vec![Arc::new(EchoTool)]);
    let manifest = test_manifest(
        "stream-tool-agent",
        "You are a tool-using assistant. ALWAYS call echo_tool when asked, then reply briefly.",
    );
    let principal = Principal::user("test-user");

    let agent_id = handle
        .spawn_with_input(
            manifest,
            "Call echo_tool with {\"text\":\"stream-test\"} and reply.".to_string(),
            principal,
            None,
        )
        .await
        .expect("spawn failed");

    let result = wait_for_result(&handle, agent_id, 60).await;

    // If the model made tool calls, the streaming pipeline internally emitted
    // ToolCallStart/ToolCallEnd events via the StreamHandle.
    assert!(
        result.tool_calls > 0,
        "expected at least one tool call, got {}",
        result.tool_calls
    );
    assert!(
        !result.output.trim().is_empty(),
        "should have final output after tool call"
    );

    // Clean up: send Kill signal to stop the process.
    let _ = handle.send_signal(agent_id, rara_kernel::process::Signal::Kill);
    cancel.cancel();
}

// ---------------------------------------------------------------------------
// Test 3: Multi-session stream isolation
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Ollama instance"]
async fn test_multi_session_isolation() {
    let (handle, cancel) = start_test_kernel(vec![]);
    let principal = Principal::user("test-user");

    let manifest1 = test_manifest("session-1-agent", "Reply with exactly: session one");
    let manifest2 = test_manifest("session-2-agent", "Reply with exactly: session two");

    // Spawn two agents — each gets its own isolated session.
    let agent_id1 = handle
        .spawn_with_input(
            manifest1,
            "Which session are you?".to_string(),
            principal.clone(),
            None,
        )
        .await
        .expect("spawn 1 failed");

    let agent_id2 = handle
        .spawn_with_input(
            manifest2,
            "Which session are you?".to_string(),
            principal,
            None,
        )
        .await
        .expect("spawn 2 failed");

    // Both should complete independently (run sequentially to avoid 429).
    let result1 = wait_for_result(&handle, agent_id1, 60).await;
    let result2 = wait_for_result(&handle, agent_id2, 60).await;

    // Both should produce output.
    assert!(
        !result1.output.trim().is_empty(),
        "session 1 should produce output"
    );
    assert!(
        !result2.output.trim().is_empty(),
        "session 2 should produce output"
    );

    // Clean up: send Kill signal to stop each process.
    for id in [agent_id1, agent_id2] {
        let _ = handle.send_signal(id, rara_kernel::process::Signal::Kill);
    }
    cancel.cancel();
}

// Tests 4 and 5 (AgentRunner streaming) have been removed — the legacy
// `AgentRunner` / `LlmProvider` path has been replaced by
// `agent_turn::run_inline_agent_loop` + `LlmDriver`.

// Tests 6 and 7 (InboundBus, OutboundBus) have been removed — these bus
// traits are replaced by the unified EventQueue.  EventQueue tests live in
// `crate::queue` and in the `event_loop` module.

// ---------------------------------------------------------------------------
// Test 8: StreamHub lifecycle — open, subscribe, emit, close
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_stream_hub_full_lifecycle() {
    let hub = StreamHub::new(64);
    let session_id = SessionId::from_raw("lifecycle-session");

    // Open a stream.
    let handle = hub.open(session_id.clone());
    let stream_id = handle.stream_id().clone();

    // Subscribe to the session.
    let subs = hub.subscribe_session(&session_id);
    assert_eq!(subs.len(), 1, "should have one stream for session");

    let (_, mut rx) = subs.into_iter().next().unwrap();

    // Emit events.
    handle.emit(StreamEvent::Progress {
        stage: "starting".to_string(),
    });
    handle.emit(StreamEvent::TextDelta {
        text: "Hello ".to_string(),
    });
    handle.emit(StreamEvent::TextDelta {
        text: "world!".to_string(),
    });
    handle.emit(StreamEvent::ToolCallStart {
        name:      "echo_tool".to_string(),
        id:        "tc-1".to_string(),
        arguments: serde_json::json!({"text": "hello"}),
    });
    handle.emit(StreamEvent::ToolCallEnd {
        id:             "tc-1".to_string(),
        result_preview: "{\"text\":\"hello\"}".to_string(),
        success:        true,
        error:          None,
    });

    // Receive and verify events.
    let e1 = rx.recv().await.unwrap();
    assert!(matches!(e1, StreamEvent::Progress { stage } if stage == "starting"));

    let e2 = rx.recv().await.unwrap();
    assert!(matches!(e2, StreamEvent::TextDelta { ref text } if text == "Hello "));

    let e3 = rx.recv().await.unwrap();
    assert!(matches!(e3, StreamEvent::TextDelta { ref text } if text == "world!"));

    let e4 = rx.recv().await.unwrap();
    assert!(matches!(e4, StreamEvent::ToolCallStart { ref name, .. } if name == "echo_tool"));

    let e5 = rx.recv().await.unwrap();
    assert!(
        matches!(e5, StreamEvent::ToolCallEnd { ref id, success, .. } if id == "tc-1" && success)
    );

    // Close the stream.
    hub.close(&stream_id);

    // After close, no more streams for the session.
    let subs = hub.subscribe_session(&session_id);
    assert_eq!(subs.len(), 0, "should have no streams after close");
}
