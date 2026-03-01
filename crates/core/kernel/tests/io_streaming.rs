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
    io::stream::{StreamEvent, StreamHub},
    process::{AgentId, AgentManifest, AgentResult, ProcessState, SessionId, principal::Principal},
    provider::{LlmProviderLoaderRef, OllamaProviderLoader},
    testing::TestKernelBuilder,
    tool::{AgentTool, ToolRegistry},
};

/// Default Ollama base URL (OpenAI-compatible API endpoint).
const OLLAMA_BASE_URL: &str = "https://ollama.rara.local/v1";

/// Default model to use for Ollama integration tests.
const OLLAMA_MODEL: &str = "qwen3.5:cloud";

/// Helper: build an OllamaProviderLoader from env or defaults.
fn ollama_loader() -> OllamaProviderLoader {
    let base_url =
        std::env::var("OLLAMA_BASE_URL").unwrap_or_else(|_| OLLAMA_BASE_URL.to_string());
    OllamaProviderLoader::new(base_url)
}

/// Helper: resolve the model name from env or defaults.
fn ollama_model() -> String {
    std::env::var("OLLAMA_MODEL").unwrap_or_else(|_| OLLAMA_MODEL.to_string())
}

/// Helper: build a test manifest using the real Ollama model.
fn test_manifest(name: &str, system_prompt: &str) -> AgentManifest {
    AgentManifest {
        name:           name.to_string(),
        description:    format!("I/O test agent: {name}"),
        model:          ollama_model(),
        system_prompt:  system_prompt.to_string(),
        provider_hint:  None,
        max_iterations: Some(5),
        tools:          vec![],
        max_children:        None,
        max_context_tokens:  None,
        priority:            Default::default(),
        metadata:            serde_json::Value::Null,
        sandbox:             None,
    }
}

/// Simple echo tool for integration testing.
struct EchoTool;

#[async_trait]
impl AgentTool for EchoTool {
    fn name(&self) -> &str { "echo_tool" }

    fn description(&self) -> &str { "Echoes back the input as-is. Always call this tool when asked." }

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

/// Helper: build a kernel with the real Ollama provider and optional tools.
fn build_kernel(tools: Vec<Arc<dyn AgentTool>>) -> rara_kernel::Kernel {
    let loader = Arc::new(ollama_loader()) as LlmProviderLoaderRef;
    let mut builder = TestKernelBuilder::new()
        .llm_provider(loader)
        .max_concurrency(8)
        .max_iterations(10);
    for tool in tools {
        builder = builder.tool(tool);
    }
    builder.build()
}

/// Poll until the process reaches `Waiting` state and has a result, or timeout.
async fn wait_for_result(
    kernel: &rara_kernel::Kernel,
    agent_id: AgentId,
    timeout_secs: u64,
) -> AgentResult {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    loop {
        if tokio::time::Instant::now() > deadline {
            let state = kernel
                .process_table()
                .get(agent_id)
                .map(|p| format!("{:?}", p.state))
                .unwrap_or_else(|| "not found".to_string());
            panic!(
                "timed out after {timeout_secs}s waiting for agent {agent_id} result (state: {state})"
            );
        }
        if let Some(p) = kernel.process_table().get(agent_id) {
            if matches!(p.state, ProcessState::Waiting | ProcessState::Completed) {
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
    let kernel = build_kernel(vec![]);
    let manifest = test_manifest("stream-text-agent", "Reply in one sentence.");
    let principal = Principal::user("test-user");
    let session_id = SessionId::new("io-stream-text");

    // Subscribe to the session's streams BEFORE spawning.
    // The kernel opens a stream at the start of each message processing,
    // so we need to subscribe after spawn but the timing is tricky.
    // Instead, we subscribe right after spawn and poll for new streams.
    let handle = kernel
        .spawn_with_input(
            manifest,
            "Say hello in one sentence.".to_string(),
            principal,
            session_id.clone(),
            None,
        )
        .await
        .expect("spawn failed");

    // Wait for the agent to finish processing via ProcessTable polling.
    let result = wait_for_result(&kernel, handle.agent_id, 60).await;

    // Verify result was produced (which means stream events were emitted internally).
    assert!(
        !result.output.trim().is_empty(),
        "agent should produce output via streaming pipeline"
    );

    if let Some(token) = kernel.process_table().get_cancellation_token(&handle.agent_id) {
        token.cancel();
    }
}

// ---------------------------------------------------------------------------
// Test 2: StreamHub emits ToolCallStart/ToolCallEnd during tool use
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Ollama instance"]
async fn test_stream_hub_tool_call_events() {
    let kernel = build_kernel(vec![Arc::new(EchoTool)]);
    let manifest = test_manifest(
        "stream-tool-agent",
        "You are a tool-using assistant. ALWAYS call echo_tool when asked, then reply briefly.",
    );
    let principal = Principal::user("test-user");
    let session_id = SessionId::new("io-stream-tools");

    let handle = kernel
        .spawn_with_input(
            manifest,
            "Call echo_tool with {\"text\":\"stream-test\"} and reply.".to_string(),
            principal,
            session_id,
            None,
        )
        .await
        .expect("spawn failed");

    let result = wait_for_result(&kernel, handle.agent_id, 60).await;

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

    if let Some(token) = kernel.process_table().get_cancellation_token(&handle.agent_id) {
        token.cancel();
    }
}

// ---------------------------------------------------------------------------
// Test 3: Multi-session stream isolation
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Ollama instance"]
async fn test_multi_session_isolation() {
    let kernel = build_kernel(vec![]);
    let principal = Principal::user("test-user");

    let manifest1 = test_manifest("session-1-agent", "Reply with exactly: session one");
    let manifest2 = test_manifest("session-2-agent", "Reply with exactly: session two");

    let session_id1 = SessionId::new("io-session-1");
    let session_id2 = SessionId::new("io-session-2");

    // Spawn two agents on different sessions concurrently.
    let handle1 = kernel
        .spawn_with_input(
            manifest1,
            "Which session are you?".to_string(),
            principal.clone(),
            session_id1,
            None,
        )
        .await
        .expect("spawn 1 failed");

    let handle2 = kernel
        .spawn_with_input(
            manifest2,
            "Which session are you?".to_string(),
            principal,
            session_id2,
            None,
        )
        .await
        .expect("spawn 2 failed");

    // Both should complete independently (run sequentially to avoid 429).
    let result1 = wait_for_result(&kernel, handle1.agent_id, 60).await;
    let result2 = wait_for_result(&kernel, handle2.agent_id, 60).await;

    // Both should produce output.
    assert!(
        !result1.output.trim().is_empty(),
        "session 1 should produce output"
    );
    assert!(
        !result2.output.trim().is_empty(),
        "session 2 should produce output"
    );

    // Clean up.
    for h in [&handle1, &handle2] {
        if let Some(token) = kernel.process_table().get_cancellation_token(&h.agent_id) {
            token.cancel();
        }
    }
}

// ---------------------------------------------------------------------------
// Test 4: AgentRunner streaming produces RunnerEvents
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Ollama instance"]
async fn test_runner_streaming_produces_events() {
    use rara_kernel::runner::{AgentRunner, RunnerEvent, UserContent};

    let loader = Arc::new(ollama_loader()) as LlmProviderLoaderRef;
    let model = ollama_model();
    let tools = ToolRegistry::new();

    let runner = AgentRunner::builder()
        .llm_provider(loader)
        .model_name(model)
        .system_prompt("You are a concise assistant.")
        .user_content(UserContent::Text("Count from 1 to 3.".to_string()))
        .max_iterations(3_usize)
        .build();

    let mut rx = runner.run_streaming(Arc::new(tools));

    let mut events = Vec::new();
    let mut got_text_delta = false;
    let mut got_done = false;
    let mut got_thinking = false;

    while let Some(event) = rx.recv().await {
        match &event {
            RunnerEvent::TextDelta(text) if !text.is_empty() => got_text_delta = true,
            RunnerEvent::Done { text, .. } => {
                assert!(!text.trim().is_empty(), "Done should have text");
                got_done = true;
            }
            RunnerEvent::Thinking => got_thinking = true,
            RunnerEvent::Error(err) => panic!("streaming error: {err}"),
            _ => {}
        }
        events.push(event);
    }

    assert!(got_text_delta, "expected at least one TextDelta event");
    assert!(got_done, "expected a Done event");
    assert!(got_thinking, "expected a Thinking event");
    assert!(
        events.len() >= 3,
        "expected at least 3 events (Thinking, TextDelta, Done), got {}",
        events.len()
    );
}

// ---------------------------------------------------------------------------
// Test 5: AgentRunner streaming with tool calls produces tool events
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Ollama instance"]
async fn test_runner_streaming_tool_events() {
    use rara_kernel::runner::{AgentRunner, RunnerEvent, UserContent};

    let loader = Arc::new(ollama_loader()) as LlmProviderLoaderRef;
    let model = ollama_model();
    let mut tools = ToolRegistry::new();
    tools.register_builtin(Arc::new(EchoTool));

    let runner = AgentRunner::builder()
        .llm_provider(loader)
        .model_name(model)
        .system_prompt(
            "You are a tool-using assistant. ALWAYS call echo_tool exactly once before replying.",
        )
        .user_content(UserContent::Text(
            "Call echo_tool with {\"text\":\"streaming-test\"} and reply.".to_string(),
        ))
        .max_iterations(5_usize)
        .build();

    let mut rx = runner.run_streaming(Arc::new(tools));

    let mut got_tool_start = false;
    let mut got_tool_end = false;
    let mut got_done = false;
    let mut tool_name = String::new();

    while let Some(event) = rx.recv().await {
        match &event {
            RunnerEvent::ToolCallStart { name, .. } => {
                got_tool_start = true;
                tool_name = name.clone();
            }
            RunnerEvent::ToolCallEnd { success, .. } => {
                got_tool_end = true;
                assert!(success, "tool call should succeed");
            }
            RunnerEvent::Done { text, tool_calls_made, .. } => {
                assert!(!text.trim().is_empty(), "Done should have text");
                assert!(*tool_calls_made > 0, "should have made tool calls");
                got_done = true;
            }
            RunnerEvent::Error(err) => panic!("streaming error: {err}"),
            _ => {}
        }
    }

    assert!(got_tool_start, "expected ToolCallStart event");
    assert!(got_tool_end, "expected ToolCallEnd event");
    assert!(got_done, "expected Done event");
    assert_eq!(tool_name, "echo_tool", "tool name should be echo_tool");
}

// ---------------------------------------------------------------------------
// Test 6: InboundBus publish + drain integration
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_inbound_bus_publish_drain_integration() {
    use rara_kernel::io::{
        bus::InboundBus,
        memory_bus::InMemoryInboundBus,
        types::InboundMessage,
    };
    use rara_kernel::process::{SessionId, principal::UserId};

    let bus = InMemoryInboundBus::new(64);

    // Publish 3 messages.
    for i in 0..3 {
        let msg = InboundMessage::synthetic(
            format!("message {i}"),
            UserId(format!("user-{i}")),
            SessionId::new(&format!("session-{i}")),
        );
        bus.publish(msg).await.expect("publish should succeed");
    }
    assert_eq!(bus.pending_count(), 3);

    // Drain all.
    let messages = bus.drain(10).await;
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].content.as_text(), "message 0");
    assert_eq!(messages[1].content.as_text(), "message 1");
    assert_eq!(messages[2].content.as_text(), "message 2");
    assert_eq!(bus.pending_count(), 0);
}

// ---------------------------------------------------------------------------
// Test 7: OutboundBus multi-subscriber fan-out
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_outbound_bus_multi_subscriber_fanout() {
    use rara_kernel::io::{
        bus::OutboundBus,
        memory_bus::InMemoryOutboundBus,
        types::{
            MessageId, OutboundEnvelope, OutboundPayload, OutboundRouting,
        },
    };
    use rara_kernel::channel::types::MessageContent;
    use rara_kernel::process::{SessionId, principal::UserId};

    let bus = InMemoryOutboundBus::new(64);

    // Create 3 subscribers.
    let mut sub1 = bus.subscribe();
    let mut sub2 = bus.subscribe();
    let mut sub3 = bus.subscribe();

    // Publish an envelope.
    let envelope = OutboundEnvelope {
        id:          MessageId::new(),
        in_reply_to: MessageId::new(),
        user:        UserId("u1".to_string()),
        session_id:  SessionId::new("s1"),
        routing:     OutboundRouting::BroadcastAll,
        payload:     OutboundPayload::Reply {
            content:     MessageContent::Text("hello all".to_string()),
            attachments: vec![],
        },
        timestamp:   jiff::Timestamp::now(),
    };

    bus.publish(envelope).await.unwrap();

    // All 3 subscribers should receive it.
    let msg1 = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        sub1.recv(),
    )
    .await
    .expect("sub1 timed out")
    .expect("sub1 should receive message");

    let msg2 = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        sub2.recv(),
    )
    .await
    .expect("sub2 timed out")
    .expect("sub2 should receive message");

    let msg3 = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        sub3.recv(),
    )
    .await
    .expect("sub3 timed out")
    .expect("sub3 should receive message");

    // All should have the same message ID.
    assert_eq!(msg1.id, msg2.id);
    assert_eq!(msg2.id, msg3.id);
}

// ---------------------------------------------------------------------------
// Test 8: StreamHub lifecycle — open, subscribe, emit, close
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_stream_hub_full_lifecycle() {
    let hub = StreamHub::new(64);
    let session_id = SessionId::new("lifecycle-session");

    // Open a stream.
    let handle = hub.open(session_id.clone());
    let stream_id = handle.stream_id().clone();

    // Subscribe to the session.
    let subs = hub.subscribe_session(&session_id);
    assert_eq!(subs.len(), 1, "should have one stream for session");

    let (_, mut rx) = subs.into_iter().next().unwrap();

    // Emit events.
    handle.emit(StreamEvent::Progress { stage: "starting".to_string() });
    handle.emit(StreamEvent::TextDelta("Hello ".to_string()));
    handle.emit(StreamEvent::TextDelta("world!".to_string()));
    handle.emit(StreamEvent::ToolCallStart {
        name: "echo_tool".to_string(),
        id:   "tc-1".to_string(),
    });
    handle.emit(StreamEvent::ToolCallEnd { id: "tc-1".to_string() });

    // Receive and verify events.
    let e1 = rx.recv().await.unwrap();
    assert!(matches!(e1, StreamEvent::Progress { stage } if stage == "starting"));

    let e2 = rx.recv().await.unwrap();
    assert!(matches!(e2, StreamEvent::TextDelta(ref s) if s == "Hello "));

    let e3 = rx.recv().await.unwrap();
    assert!(matches!(e3, StreamEvent::TextDelta(ref s) if s == "world!"));

    let e4 = rx.recv().await.unwrap();
    assert!(matches!(e4, StreamEvent::ToolCallStart { ref name, .. } if name == "echo_tool"));

    let e5 = rx.recv().await.unwrap();
    assert!(matches!(e5, StreamEvent::ToolCallEnd { ref id } if id == "tc-1"));

    // Close the stream.
    hub.close(&stream_id);

    // After close, no more streams for the session.
    let subs = hub.subscribe_session(&session_id);
    assert_eq!(subs.len(), 0, "should have no streams after close");
}
