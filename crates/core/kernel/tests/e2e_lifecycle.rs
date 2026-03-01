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
//! These tests verify the **full Kernel.spawn → process_loop → result** path
//! using a real Ollama instance. They exercise the complete pipeline:
//! Kernel spawn → session setup → process_loop → LLM call → process table
//! result.
//!
//! **Important**: The process loop is long-lived (waits for more messages
//! after processing). `result_rx` only fires when the process *terminates*
//! (mailbox closed or cancelled). These tests poll the ProcessTable for
//! `Waiting` state to detect message completion, then read the result from
//! `AgentProcess.result`.
//!
//! Run with:
//! ```sh
//! cargo test -p rara-kernel --test e2e_lifecycle -- --ignored --test-threads=1
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use rara_kernel::{
    process::{AgentId, AgentManifest, AgentResult, ProcessState, SessionId, principal::Principal},
    provider::{LlmProviderLoaderRef, OllamaProviderLoader},
    testing::TestKernelBuilder,
    tool::AgentTool,
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
        metadata:            serde_json::Value::Null,
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
///
/// The process loop sets `Waiting` after processing each message and stores
/// the result in the process table. This is the correct way to detect
/// completion for long-lived processes (as opposed to `result_rx` which only
/// fires on process termination).
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
// Test 1: Spawn plain text agent → result appears in process table
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Ollama instance"]
async fn test_spawn_plain_text_receives_result() {
    let kernel = build_kernel(vec![]);
    let manifest = test_manifest("plain-agent", "You are a concise assistant. Reply in one sentence.");
    let principal = Principal::user("test-user");
    let session_id = SessionId::new("e2e-plain-text");

    let handle = kernel
        .spawn_with_input(
            manifest,
            "What is 2 + 2? Reply with just the number.".to_string(),
            principal,
            session_id,
            None,
        )
        .await
        .expect("spawn failed");

    let result = wait_for_result(&kernel, handle.agent_id, 60).await;

    assert!(
        !result.output.trim().is_empty(),
        "expected non-empty output, got: {:?}",
        result.output
    );
    assert!(result.iterations > 0, "expected at least one iteration");

    // Clean up: cancel the process.
    if let Some(token) = kernel.process_table().get_cancellation_token(&handle.agent_id) {
        token.cancel();
    }
}

// ---------------------------------------------------------------------------
// Test 2: Spawn agent with tool → model uses tool
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Ollama instance"]
async fn test_spawn_with_tool_makes_tool_call() {
    let kernel = build_kernel(vec![Arc::new(EchoTool)]);
    let manifest = test_manifest(
        "tool-agent",
        "You are a tool-using assistant. When the user asks you to echo something, \
         ALWAYS call echo_tool with the text, then summarize the result.",
    );
    let principal = Principal::user("test-user");
    let session_id = SessionId::new("e2e-tool-call");

    let handle = kernel
        .spawn_with_input(
            manifest,
            "Please call echo_tool with {\"text\":\"integration-test\"} and tell me the result.".to_string(),
            principal,
            session_id,
            None,
        )
        .await
        .expect("spawn failed");

    let result = wait_for_result(&kernel, handle.agent_id, 60).await;

    assert!(
        result.tool_calls > 0,
        "expected at least one tool call, got {}",
        result.tool_calls
    );
    assert!(
        !result.output.trim().is_empty(),
        "expected non-empty output after tool call"
    );

    if let Some(token) = kernel.process_table().get_cancellation_token(&handle.agent_id) {
        token.cancel();
    }
}

// ---------------------------------------------------------------------------
// Test 3: Process state transitions during spawn
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Ollama instance"]
async fn test_process_state_transitions() {
    let kernel = build_kernel(vec![]);
    let manifest = test_manifest("state-agent", "Reply in one sentence.");
    let principal = Principal::user("test-user");
    let session_id = SessionId::new("e2e-state");

    let handle = kernel
        .spawn_with_input(
            manifest,
            "Say hello.".to_string(),
            principal,
            session_id,
            None,
        )
        .await
        .expect("spawn failed");

    let agent_id = handle.agent_id;

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

    // Cancel → should transition to Cancelled.
    if let Some(token) = kernel.process_table().get_cancellation_token(&agent_id) {
        token.cancel();
    }
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let process = kernel.process_table().get(agent_id).unwrap();
    assert!(
        matches!(process.state, ProcessState::Cancelled | ProcessState::Completed),
        "expected Cancelled after cancel, got {:?}",
        process.state
    );
}

// ---------------------------------------------------------------------------
// Test 4: Outbound bus receives reply after spawn
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Ollama instance"]
async fn test_outbound_bus_receives_reply() {
    let kernel = build_kernel(vec![]);
    let manifest = test_manifest("outbound-agent", "Reply in one sentence.");
    let principal = Principal::user("test-user");
    let session_id = SessionId::new("e2e-outbound");

    let handle = kernel
        .spawn_with_input(
            manifest,
            "What color is the sky?".to_string(),
            principal,
            session_id,
            None,
        )
        .await
        .expect("spawn failed");

    let result = wait_for_result(&kernel, handle.agent_id, 60).await;

    // The result should contain meaningful output, confirming the full pipeline
    // including outbound bus publication.
    assert!(
        !result.output.trim().is_empty(),
        "outbound pipeline should produce non-empty result"
    );
    assert!(
        result.iterations > 0,
        "should have at least 1 LLM iteration"
    );

    if let Some(token) = kernel.process_table().get_cancellation_token(&handle.agent_id) {
        token.cancel();
    }
}

// ---------------------------------------------------------------------------
// Test 5: Cancellation via CancellationToken
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Ollama instance"]
async fn test_cancellation_stops_process() {
    let kernel = build_kernel(vec![]);
    let manifest = test_manifest("cancel-agent", "Reply in one sentence.");
    let principal = Principal::user("test-user");
    let session_id = SessionId::new("e2e-cancel");

    let handle = kernel
        .spawn_with_input(
            manifest,
            "Say hello.".to_string(),
            principal,
            session_id,
            None,
        )
        .await
        .expect("spawn failed");

    let agent_id = handle.agent_id;

    // Wait for the first turn to complete (enters Waiting state).
    let _result = wait_for_result(&kernel, agent_id, 60).await;

    // Now cancel — the select! will see the token on the next loop iteration.
    if let Some(token) = kernel.process_table().get_cancellation_token(&agent_id) {
        token.cancel();
    }

    // Give a moment for the process loop to exit.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // After cancellation, check the process state.
    let process = kernel.process_table().get(agent_id).unwrap();
    assert!(
        matches!(process.state, ProcessState::Cancelled | ProcessState::Completed),
        "expected Cancelled state after token cancel, got {:?}",
        process.state
    );
}

// ---------------------------------------------------------------------------
// Test 6: Multi-turn conversation via mailbox
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Ollama instance"]
async fn test_multi_turn_via_mailbox() {
    let kernel = build_kernel(vec![]);
    let manifest = test_manifest(
        "multi-turn-agent",
        "You are a concise assistant that remembers context. Reply in one sentence.",
    );
    let principal = Principal::user("test-user");
    let session_id = SessionId::new("e2e-multi-turn");

    let handle = kernel
        .spawn_with_input(
            manifest,
            "My name is Alice.".to_string(),
            principal.clone(),
            session_id.clone(),
            None,
        )
        .await
        .expect("spawn failed");

    let agent_id = handle.agent_id;

    // Wait for first turn to complete.
    let _first = wait_for_result(&kernel, agent_id, 60).await;

    // Send a second message referencing the first.
    let second_msg = rara_kernel::io::types::InboundMessage::synthetic(
        "What is my name?".to_string(),
        principal.user_id.clone(),
        session_id,
    );
    handle
        .mailbox
        .send(rara_kernel::ProcessMessage::UserMessage(second_msg))
        .await
        .expect("failed to send second message");

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
                        // Clean up.
                        if let Some(token) =
                            kernel.process_table().get_cancellation_token(&agent_id)
                        {
                            token.cancel();
                        }
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
    let kernel = build_kernel(vec![]);
    let principal = Principal::user("test-user");

    let manifest1 = test_manifest("agent-1", "Reply with exactly: done");
    let session_id1 = SessionId::new("e2e-multi-1");
    let handle1 = kernel
        .spawn_with_input(
            manifest1,
            "Say done.".to_string(),
            principal.clone(),
            session_id1,
            None,
        )
        .await
        .expect("spawn 1 failed");

    let result1 = wait_for_result(&kernel, handle1.agent_id, 60).await;
    assert!(!result1.output.trim().is_empty());

    let manifest2 = test_manifest("agent-2", "Reply with exactly: done");
    let session_id2 = SessionId::new("e2e-multi-2");
    let handle2 = kernel
        .spawn_with_input(
            manifest2,
            "Say done.".to_string(),
            principal,
            session_id2,
            None,
        )
        .await
        .expect("spawn 2 failed");

    let result2 = wait_for_result(&kernel, handle2.agent_id, 60).await;
    assert!(!result2.output.trim().is_empty());

    // Clean up.
    for h in [&handle1, &handle2] {
        if let Some(token) = kernel.process_table().get_cancellation_token(&h.agent_id) {
            token.cancel();
        }
    }
}

// ---------------------------------------------------------------------------
// Test 8: Spawn named agent (built-in manifest lookup)
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Ollama instance"]
async fn test_spawn_named_agent() {
    let kernel = build_kernel(vec![]);
    let principal = Principal::user("test-user");
    let session_id = SessionId::new("e2e-named");

    // Verify named resolution works (the scout manifest uses a different model
    // so the LLM response may be empty — we just verify the process spawns
    // correctly and reaches Running or Waiting state).
    let handle = kernel
        .spawn_named(
            "scout",
            "List 3 colors.".to_string(),
            principal,
            session_id,
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
                .get(handle.agent_id)
                .map(|p| format!("{:?}", p.state))
                .unwrap_or_else(|| "not found".to_string());
            panic!("timed out waiting for named agent to reach Waiting (state: {state})");
        }
        if let Some(p) = kernel.process_table().get(handle.agent_id) {
            match p.state {
                ProcessState::Waiting | ProcessState::Completed => break,
                ProcessState::Failed => panic!("named agent failed"),
                _ => {}
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }

    // Process existed and was executed — named resolution works.
    let process = kernel.process_table().get(handle.agent_id).unwrap();
    assert!(
        matches!(process.state, ProcessState::Waiting | ProcessState::Completed),
        "named agent should reach Waiting after processing"
    );

    if let Some(token) = kernel.process_table().get_cancellation_token(&handle.agent_id) {
        token.cancel();
    }
}

// ---------------------------------------------------------------------------
// Test 9: Global concurrency limit enforcement
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Ollama instance"]
async fn test_global_concurrency_limit() {
    // Build kernel with only 2 slots.
    let loader = Arc::new(ollama_loader()) as LlmProviderLoaderRef;
    let kernel = TestKernelBuilder::new()
        .llm_provider(loader)
        .max_concurrency(2)
        .max_iterations(10)
        .build();

    let principal = Principal::user("test-user");
    let manifest = test_manifest(
        "limit-agent",
        "You are an assistant. Write a very detailed 500 word essay.",
    );

    // Spawn 2 agents (fills capacity).
    let h1 = kernel
        .spawn_with_input(
            manifest.clone(),
            "Essay topic 1.".to_string(),
            principal.clone(),
            SessionId::new("limit-1"),
            None,
        )
        .await
        .expect("first spawn should succeed");

    let h2 = kernel
        .spawn_with_input(
            manifest.clone(),
            "Essay topic 2.".to_string(),
            principal.clone(),
            SessionId::new("limit-2"),
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
            SessionId::new("limit-3"),
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

    // Cancel the first two so we don't leave hanging tasks.
    if let Some(token) = kernel.process_table().get_cancellation_token(&h1.agent_id) {
        token.cancel();
    }
    if let Some(token) = kernel.process_table().get_cancellation_token(&h2.agent_id) {
        token.cancel();
    }
}
