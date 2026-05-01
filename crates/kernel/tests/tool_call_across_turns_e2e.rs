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

//! E2E (lane 2 — scripted LLM) — tool wiring across turns.
//!
//! Asserts the cross-turn invariant the deleted `real_tape_flow` soak was
//! indirectly probing (issue #2016, concern 3): a tool call emitted in
//! turn N must produce a `ToolCall` + `ToolResult` row on the tape, AND
//! the `TapeService::build_llm_context` path that the kernel uses to
//! rebuild the prompt for turn N+1 must surface the tool result.
//!
//! We don't drive a literal second turn here — that would require either
//! re-spawning on the same session key (the kernel's session lifecycle
//! is the wrong unit-of-test for this concern) or a flaky inbound-message
//! retry. The load-bearing invariant is "turn 1's persisted side-effects
//! materialise into the context turn 2 would see", which is exactly what
//! `build_llm_context` produces. Asserting on its output gives a
//! deterministic regression gate without coupling to spawn/finalisation
//! timing.
//!
//! Determinism: two scripted LLM responses (iter 0 → tool call,
//! iter 1 → plain text) plus a `FakeTool` that returns a fixed payload.
//! No real LLM, no network, no external URL. Runs in well under a second.

use std::{sync::OnceLock, time::Duration};

use rara_kernel::{
    agent::{AgentManifest, AgentRole},
    identity::Principal,
    llm::{CompletionResponse, StopReason, ToolCallRequest, Usage},
    memory::TapEntryKind,
    testing::{FakeTool, TestKernelBuilder, scripted_response},
};
use serde_json::{Value, json};

/// Redirect `rara_paths::config_dir()` / `workspace_dir()` to a process-wide
/// tempdir so the kernel doesn't try to create `~/.config/rara/workspace`
/// on Linux ARC runners where `$HOME` is read-only. Mirrors the helper in
/// `whitespace_intermediate_tape_e2e.rs`.
fn ensure_test_paths_isolated() {
    static SHARED_TEST_CONFIG: OnceLock<tempfile::TempDir> = OnceLock::new();
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        let shared = SHARED_TEST_CONFIG
            .get_or_init(|| tempfile::tempdir().expect("create shared test config dir"));
        let _ = std::panic::catch_unwind(|| {
            rara_paths::set_custom_config_dir(shared.path());
        });
    });
}

/// Scripted turn-1 response: emits a single tool call, then yields back to
/// the agent loop with `StopReason::ToolCalls` so the kernel will dispatch
/// the tool, fold its result back into context, and ask the LLM again.
fn first_turn_with_tool_call() -> CompletionResponse {
    CompletionResponse {
        content:           Some(String::new()),
        reasoning_content: None,
        tool_calls:        vec![ToolCallRequest {
            id:        "call-cross-turn-1".to_string(),
            name:      "fake-tool".to_string(),
            arguments: r#"{"q":"first"}"#.to_string(),
        }],
        stop_reason:       StopReason::ToolCalls,
        usage:             Some(Usage {
            prompt_tokens:     8,
            completion_tokens: 4,
            total_tokens:      12,
        }),
        model:             "scripted".to_string(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tool_call_in_turn_one_is_recorded_and_surfaces_in_rebuilt_context() {
    ensure_test_paths_isolated();
    let tmp = tempfile::tempdir().expect("tempdir");

    // Distinct, recognisable payload so we can assert it surfaces in the
    // turn-2 prompt context unambiguously.
    let tool_payload = json!({"answer": "shanghai-tokyo-flight-2768"});
    let fake_tool = std::sync::Arc::new(FakeTool::new("fake-tool", vec![tool_payload.clone()]));

    let manifest = AgentManifest {
        name:                   "test-agent".to_string(),
        role:                   AgentRole::Chat,
        description:            "Test agent".to_string(),
        model:                  Some("scripted-model".to_string()),
        system_prompt:          "You are a test agent.".to_string(),
        soul_prompt:            None,
        provider_hint:          Some("scripted".to_string()),
        max_iterations:         Some(4),
        tools:                  vec!["fake-tool".to_string().into()],
        excluded_tools:         vec![],
        max_children:           None,
        max_context_tokens:     None,
        priority:               Default::default(),
        metadata:               serde_json::Value::Null,
        sandbox:                None,
        default_execution_mode: None,
        tool_call_limit:        None,
        worker_timeout_secs:    None,
        max_continuations:      None,
        max_output_chars:       None,
    };

    // Two scripted iterations on the single turn:
    //   iter 0 → tool call
    //   iter 1 → plain "done" (after tool result is folded into context)
    // The agent loop continues after `StopReason::ToolCalls` to give the
    // LLM a chance to react to the tool result; only iter 1's `Stop`
    // ends the turn.
    let tk = TestKernelBuilder::new(tmp.path())
        .manifest(manifest)
        .with_tool(fake_tool.clone())
        .responses(vec![first_turn_with_tool_call(), scripted_response("done")])
        .build()
        .await;

    // Spawn the agent for the single turn. The watcher fires on completion.
    let principal = Principal::lookup("test");
    let (session_key, waiter) = tk
        .spawn_named_watching("test-agent", "kick off the turn", principal)
        .await
        .expect("spawn agent");
    waiter
        .wait(Duration::from_secs(30))
        .await
        .expect("turn metrics");

    // Assertion A: the turn trace records the tool call.
    let turns = tk.handle.get_process_turns(session_key);
    assert_eq!(turns.len(), 1, "exactly one turn after spawn");
    let turn = &turns[0];
    assert!(turn.success, "turn should succeed: {:?}", turn.error);
    let tool_calls: Vec<_> = turn
        .iterations
        .iter()
        .flat_map(|iter| iter.tool_calls.iter())
        .collect();
    assert_eq!(
        tool_calls.len(),
        1,
        "turn should record exactly one tool call, got: {tool_calls:?}"
    );
    assert_eq!(tool_calls[0].name, "fake-tool");

    // Assertion B: tape has a ToolCall row + a ToolResult row carrying the
    // exact scripted payload. This is the persistence half — without it
    // the next turn's context rebuild would have nothing to fold in.
    let tape_name = session_key.to_string();
    let entries_after_one = tk
        .handle
        .tape()
        .entries(&tape_name)
        .await
        .expect("read tape after turn 1");
    let tool_call_rows: Vec<_> = entries_after_one
        .iter()
        .filter(|e| e.kind == TapEntryKind::ToolCall)
        .collect();
    let tool_result_rows: Vec<_> = entries_after_one
        .iter()
        .filter(|e| e.kind == TapEntryKind::ToolResult)
        .collect();
    assert_eq!(
        tool_call_rows.len(),
        1,
        "expected one ToolCall row, got entries: {:#?}",
        entries_after_one
            .iter()
            .map(|e| (e.id, e.kind))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        tool_result_rows.len(),
        1,
        "expected one ToolResult row, got entries: {:#?}",
        entries_after_one
            .iter()
            .map(|e| (e.id, e.kind))
            .collect::<Vec<_>>()
    );
    let recorded_payload = tool_result_rows[0]
        .payload
        .get("result")
        .or_else(|| tool_result_rows[0].payload.get("output"))
        .or_else(|| tool_result_rows[0].payload.get("content"))
        .cloned()
        .unwrap_or_else(|| tool_result_rows[0].payload.clone());
    let recorded_str = serde_json::to_string(&recorded_payload).expect("serialize tool result");
    assert!(
        recorded_str.contains("shanghai-tokyo-flight-2768"),
        "tool result row should carry the scripted payload, got: {recorded_str}"
    );

    // Assertion C — the load-bearing one: rebuilding the LLM context the
    // same way the kernel does for turn N+1 (`TapeService::build_llm_context`)
    // surfaces the turn-1 tool result payload. This is the cross-turn
    // wiring invariant: ToolCall + ToolResult rows persisted in turn 1
    // make it back into the prompt the LLM would see in turn 2.
    let rebuilt_messages = tk
        .handle
        .tape()
        .build_llm_context(&tape_name)
        .await
        .expect("rebuild messages from tape");
    let rebuilt_str = serde_json::to_string(&rebuilt_messages).expect("serialize rebuilt context");
    assert!(
        rebuilt_str.contains("shanghai-tokyo-flight-2768"),
        "rebuilt LLM context for turn 2 must include the turn-1 tool result; got: {rebuilt_str}"
    );

    // Sanity check: the FakeTool was invoked exactly once with the
    // scripted arguments — guards against accidental retries that would
    // mask a real wiring break.
    let captured: Vec<Value> = fake_tool.captured_inputs();
    assert_eq!(
        captured.len(),
        1,
        "fake-tool should be invoked exactly once, captured: {captured:?}"
    );

    tk.shutdown();
}
