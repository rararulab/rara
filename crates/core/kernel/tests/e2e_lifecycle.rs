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

//! E2E lifecycle integration tests for the agent OS kernel.
//!
//! These tests verify the **full Kernel.spawn → event loop → result** path
//! using a real Ollama instance. They exercise the complete pipeline:
//! Kernel spawn → session setup → event loop → LLM call → process table
//! result.
//!
//! **Important**: The event loop processes KernelEvents from the EventQueue.
//! `spawn_with_input` pushes a SpawnAgent event and waits for the reply.
//! These tests poll the ProcessTable for `Waiting` state to detect message
//! completion, then read the result from `AgentProcess.result`.
//!
//! Run with:
//! ```sh
//! cargo test -p rara-kernel --test e2e_lifecycle -- --ignored --test-threads=1
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;
use rara_kernel::{
    process::{AgentId, AgentManifest, AgentResult, ProcessState, principal::Principal},
    provider::{LlmProviderLoaderRef, OllamaProviderLoader},
    testing::TestKernelBuilder,
    tool::AgentTool,
    Kernel,
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
        description:    format!("E2E test agent: {name}"),
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

/// Helper: build a kernel with the real Ollama provider and optional tools,
/// start the event loop, and return the Arc<Kernel> + CancellationToken.
fn build_kernel(tools: Vec<Arc<dyn AgentTool>>) -> (Arc<Kernel>, CancellationToken) {
    let loader = Arc::new(ollama_loader()) as LlmProviderLoaderRef;
    let mut builder = TestKernelBuilder::new()
        .llm_provider(loader)
        .max_concurrency(8)
        .max_iterations(10);
    for tool in tools {
        builder = builder.tool(tool);
    }
    let kernel = builder.build();
    let cancel = CancellationToken::new();
    let arc = kernel.start(cancel.clone());
    (arc, cancel)
}

/// Poll until the process reaches `Waiting` state and has a result, or timeout.
async fn wait_for_result(
    kernel: &Kernel,
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
// Test 1: Spawn plain text agent -> result appears in process table
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Ollama instance"]
async fn test_spawn_plain_text_receives_result() {
    let (kernel, cancel) = build_kernel(vec![]);
    let manifest = test_manifest("plain-agent", "You are a concise assistant. Reply in one sentence.");
    let principal = Principal::user("test-user");

    let agent_id = kernel
        .spawn_with_input(
            manifest,
            "What is 2 + 2? Reply with just the number.".to_string(),
            principal,
            None,
        )
        .await
        .expect("spawn failed");

    let result = wait_for_result(&kernel, agent_id, 60).await;

    assert!(
        !result.output.trim().is_empty(),
        "expected non-empty output, got: {:?}",
        result.output
    );
    assert!(result.iterations > 0, "expected at least one iteration");

    cancel.cancel();
}

// ---------------------------------------------------------------------------
// Test 2: Spawn agent with tool -> model uses tool
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Ollama instance"]
async fn test_spawn_with_tool_makes_tool_call() {
    let (kernel, cancel) = build_kernel(vec![Arc::new(EchoTool)]);
    let manifest = test_manifest(
        "tool-agent",
        "You are a tool-using assistant. When the user asks you to echo something, \
         ALWAYS call echo_tool with the text, then summarize the result.",
    );
    let principal = Principal::user("test-user");

    let agent_id = kernel
        .spawn_with_input(
            manifest,
            "Please call echo_tool with {\"text\":\"integration-test\"} and tell me the result.".to_string(),
            principal,
            None,
        )
        .await
        .expect("spawn failed");

    let result = wait_for_result(&kernel, agent_id, 60).await;

    assert!(
        result.tool_calls > 0,
        "expected at least one tool call, got {}",
        result.tool_calls
    );
    assert!(
        !result.output.trim().is_empty(),
        "expected non-empty output after tool call"
    );

    cancel.cancel();
}

// ---------------------------------------------------------------------------
// Test 3: Process state transitions during spawn
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Ollama instance"]
async fn test_process_state_transitions() {
    let (kernel, cancel) = build_kernel(vec![]);
    let manifest = test_manifest("state-agent", "Reply in one sentence.");
    let principal = Principal::user("test-user");

    let agent_id = kernel
        .spawn_with_input(
            manifest,
            "Say hello.".to_string(),
            principal,
            None,
        )
        .await
        .expect("spawn failed");

    // Immediately after spawn, process should exist in the table.
    let process = kernel.process_table().get(agent_id);
    assert!(process.is_some(), "process should exist in table");

    // Wait for message processing to complete.
    let _result = wait_for_result(&kernel, agent_id, 60).await;

    // After processing, state should be Waiting (waiting for next message).
    let process = kernel.process_table().get(agent_id).unwrap();
    assert!(
        matches!(process.state, ProcessState::Waiting),
        "expected Waiting state after processing, got {:?}",
        process.state
    );

    cancel.cancel();
}

// ---------------------------------------------------------------------------
// Test 4: Egress delivers reply after spawn
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Ollama instance"]
async fn test_egress_delivers_reply() {
    let (kernel, cancel) = build_kernel(vec![]);
    let manifest = test_manifest("outbound-agent", "Reply in one sentence.");
    let principal = Principal::user("test-user");

    let agent_id = kernel
        .spawn_with_input(
            manifest,
            "What color is the sky?".to_string(),
            principal,
            None,
        )
        .await
        .expect("spawn failed");

    let result = wait_for_result(&kernel, agent_id, 60).await;

    assert!(
        !result.output.trim().is_empty(),
        "egress pipeline should produce non-empty result"
    );
    assert!(
        result.iterations > 0,
        "should have at least 1 LLM iteration"
    );

    cancel.cancel();
}

// ---------------------------------------------------------------------------
// Test 5: Cancellation via CancellationToken
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Ollama instance"]
async fn test_cancellation_stops_process() {
    let (kernel, cancel) = build_kernel(vec![]);
    let manifest = test_manifest("cancel-agent", "Reply in one sentence.");
    let principal = Principal::user("test-user");

    let agent_id = kernel
        .spawn_with_input(
            manifest,
            "Say hello.".to_string(),
            principal,
            None,
        )
        .await
        .expect("spawn failed");

    // Wait for the first turn to complete (enters Waiting state).
    let _result = wait_for_result(&kernel, agent_id, 60).await;

    // Cancel via Kill signal through the event queue.
    let _ = kernel.event_queue().try_push(
        rara_kernel::unified_event::KernelEvent::SendSignal {
            target: agent_id,
            signal: rara_kernel::process::Signal::Kill,
        },
    );

    // Give a moment for the event loop to process.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let process = kernel.process_table().get(agent_id).unwrap();
    assert!(
        matches!(process.state, ProcessState::Cancelled | ProcessState::Completed),
        "expected Cancelled state after token cancel, got {:?}",
        process.state
    );

    cancel.cancel();
}

// ---------------------------------------------------------------------------
// Test 6: Multi-turn conversation via EventQueue
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Ollama instance"]
async fn test_multi_turn_via_event_queue() {
    let (kernel, cancel) = build_kernel(vec![]);
    let manifest = test_manifest(
        "multi-turn-agent",
        "You are a concise assistant that remembers context. Reply in one sentence.",
    );
    let principal = Principal::user("test-user");

    let agent_id = kernel
        .spawn_with_input(
            manifest,
            "My name is Alice.".to_string(),
            principal.clone(),
            None,
        )
        .await
        .expect("spawn failed");

    // Wait for first turn to complete.
    let _first = wait_for_result(&kernel, agent_id, 60).await;

    // Look up the process's own session for routing the second message.
    let process_session = kernel
        .process_table()
        .get(agent_id)
        .expect("process should exist")
        .session_id
        .clone();

    // Send a second message via the event queue, addressed to the agent by name.
    let second_msg = rara_kernel::io::types::InboundMessage::synthetic_to(
        "What is my name?".to_string(),
        principal.user_id.clone(),
        process_session,
        "multi-turn-agent".to_string(),
    );
    kernel
        .event_queue()
        .push(rara_kernel::unified_event::KernelEvent::UserMessage(second_msg))
        .await
        .expect("failed to push second message");

    // Wait for second turn to complete — result should mention "Alice".
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        if tokio::time::Instant::now() > deadline {
            panic!("timed out waiting for second turn");
        }
        if let Some(p) = kernel.process_table().get(agent_id) {
            if matches!(p.state, ProcessState::Waiting) {
                if let Some(ref result) = p.result {
                    if result.output.to_lowercase().contains("alice") {
                        cancel.cancel();
                        return;
                    }
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

// ---------------------------------------------------------------------------
// Test 7: Multiple agents on different sessions
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Ollama instance"]
async fn test_multiple_agents_different_sessions() {
    let (kernel, cancel) = build_kernel(vec![]);
    let principal = Principal::user("test-user");

    let manifest1 = test_manifest("agent-1", "Reply with exactly: done");
    let id1 = kernel
        .spawn_with_input(
            manifest1,
            "Say done.".to_string(),
            principal.clone(),
            None,
        )
        .await
        .expect("spawn 1 failed");

    let result1 = wait_for_result(&kernel, id1, 60).await;
    assert!(!result1.output.trim().is_empty());

    let manifest2 = test_manifest("agent-2", "Reply with exactly: done");
    let id2 = kernel
        .spawn_with_input(
            manifest2,
            "Say done.".to_string(),
            principal,
            None,
        )
        .await
        .expect("spawn 2 failed");

    let result2 = wait_for_result(&kernel, id2, 60).await;
    assert!(!result2.output.trim().is_empty());

    cancel.cancel();
}

// ---------------------------------------------------------------------------
// Test 8: Spawn named agent (built-in manifest lookup)
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Ollama instance"]
async fn test_spawn_named_agent() {
    let (kernel, cancel) = build_kernel(vec![]);
    let principal = Principal::user("test-user");

    let agent_id = kernel
        .spawn_named(
            "scout",
            "List 3 colors.".to_string(),
            principal,
            None,
        )
        .await
        .expect("spawn_named failed");

    // Wait for the process to reach Waiting (turn processed, even if empty).
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        if tokio::time::Instant::now() > deadline {
            let state = kernel
                .process_table()
                .get(agent_id)
                .map(|p| format!("{:?}", p.state))
                .unwrap_or_else(|| "not found".to_string());
            panic!("timed out waiting for named agent to reach Waiting (state: {state})");
        }
        if let Some(p) = kernel.process_table().get(agent_id) {
            match p.state {
                ProcessState::Waiting | ProcessState::Completed => break,
                ProcessState::Failed => panic!("named agent failed"),
                _ => {}
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }

    let process = kernel.process_table().get(agent_id).unwrap();
    assert!(
        matches!(process.state, ProcessState::Waiting | ProcessState::Completed),
        "named agent should reach Waiting after processing"
    );

    cancel.cancel();
}

// ---------------------------------------------------------------------------
// Test 9: Global concurrency limit enforcement
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Ollama instance"]
async fn test_global_concurrency_limit() {
    let loader = Arc::new(ollama_loader()) as LlmProviderLoaderRef;
    let kernel = TestKernelBuilder::new()
        .llm_provider(loader)
        .max_concurrency(2)
        .max_iterations(10)
        .build();

    let cancel = CancellationToken::new();
    let kernel = kernel.start(cancel.clone());

    let principal = Principal::user("test-user");
    let manifest = test_manifest(
        "limit-agent",
        "You are an assistant. Write a very detailed 500 word essay.",
    );

    // Spawn 2 agents (fills capacity).
    let id1 = kernel
        .spawn_with_input(
            manifest.clone(),
            "Essay topic 1.".to_string(),
            principal.clone(),
            None,
        )
        .await
        .expect("first spawn should succeed");

    let id2 = kernel
        .spawn_with_input(
            manifest.clone(),
            "Essay topic 2.".to_string(),
            principal.clone(),
            None,
        )
        .await
        .expect("second spawn should succeed");

    // Third spawn should fail (capacity exhausted).
    let h3 = kernel
        .spawn_with_input(
            manifest,
            "Essay topic 3.".to_string(),
            principal,
            None,
        )
        .await;

    assert!(
        h3.is_err(),
        "third spawn should fail due to concurrency limit"
    );
    let err = h3.unwrap_err().to_string();
    assert!(
        err.contains("concurrency limit"),
        "error should mention concurrency limit, got: {err}"
    );

    // Suppress unused variable warnings.
    let _ = (id1, id2);
    cancel.cancel();
}
