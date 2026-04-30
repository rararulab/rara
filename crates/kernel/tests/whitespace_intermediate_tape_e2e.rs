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

//! E2E (lane 2 — scripted LLM) for issue #1979.
//!
//! A reasoning-capable model can finalize an intermediate iteration with
//! whitespace-only `content` while routing all real output to
//! `reasoning_content`. Before this fix the agent loop persisted that as
//! `{role: "assistant", content: "\n"}` to the tape — the UI rendered an
//! empty bubble and the next turn's context rebuild fed the whitespace
//! row back to the model.
//!
//! This test scripts that exact shape via [`ScriptedLlmDriver`] (whitespace
//! `content` + non-empty `reasoning_content` + a tool call on iteration 1,
//! plain stop on iteration 2) and asserts that the tape contains **no**
//! `Message` row with whitespace-only `content` for the affected turn.
//! The cascade-tick boundary is preserved through the `ToolCall` row.

use std::{sync::OnceLock, time::Duration};

use rara_kernel::{
    identity::Principal,
    llm::{CompletionResponse, StopReason, ToolCallRequest, Usage},
    memory::TapEntryKind,
    testing::{FakeTool, TestKernelBuilder, scripted_response},
};
use serde_json::json;

/// Redirect `rara_paths::config_dir()` / `workspace_dir()` to a process-wide
/// tempdir so the `Kernel::new` initialisation path doesn't attempt to create
/// `~/.config/rara/workspace`. CI runners (`/home/runner/.config`) aren't
/// writable by the test process, which surfaces as a `workspace_dir` panic in
/// `rara_paths` on the first build.
///
/// The `OnceLock<TempDir>` is deliberately never dropped — it must outlive the
/// test because `rara_paths`'s own `OnceLock`s cache the path and never
/// re-read.
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

/// Build a scripted response that mimics a reasoning model finalizing
/// whitespace `content` while routing all tokens to `reasoning_content`,
/// and emitting one valid tool call. The resulting iteration is the
/// exact shape that produced the empty assistant bubbles in
/// `315bad61f8c34b137221bd6ec086597c__d6e905d9-fd62-41ca-8918-97b37276f534.
/// jsonl` (see issue #1979 evidence).
fn whitespace_with_reasoning_and_tool_call() -> CompletionResponse {
    CompletionResponse {
        content:           Some("\n".to_string()),
        reasoning_content: Some("internal chain of thought".to_string()),
        tool_calls:        vec![ToolCallRequest {
            id:        "call-whitespace-1".to_string(),
            name:      "fake-tool".to_string(),
            arguments: "{}".to_string(),
        }],
        stop_reason:       StopReason::ToolCalls,
        usage:             Some(Usage {
            prompt_tokens:     10,
            completion_tokens: 55,
            total_tokens:      65,
        }),
        model:             "scripted".to_string(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn whitespace_intermediate_iteration_does_not_pollute_tape() {
    ensure_test_paths_isolated();
    let tmp = tempfile::tempdir().expect("tempdir");

    // Iteration 1: whitespace content + reasoning + tool call (the exact
    // shape that used to write `content: "\n"` to tape).
    // Iteration 2: plain stop response so the agent loop terminates
    // cleanly without hitting the terminal empty-turn rejection.
    let manifest = rara_kernel::agent::AgentManifest {
        name:                   "test-agent".to_string(),
        role:                   rara_kernel::agent::AgentRole::Chat,
        description:            "Test agent".to_string(),
        model:                  Some("scripted-model".to_string()),
        system_prompt:          "You are a test agent.".to_string(),
        soul_prompt:            None,
        provider_hint:          Some("scripted".to_string()),
        max_iterations:         Some(3),
        // Allow the fake tool to be visible to the agent loop.
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

    let fake_tool = std::sync::Arc::new(FakeTool::new(
        "fake-tool",
        vec![json!({"ok": true, "result": "tool-output"})],
    ));

    let tk = TestKernelBuilder::new(tmp.path())
        .manifest(manifest)
        .with_tool(fake_tool)
        .responses(vec![
            whitespace_with_reasoning_and_tool_call(),
            scripted_response("done"),
        ])
        .build()
        .await;

    let principal = Principal::lookup("test");
    let session_key = tk
        .handle
        .spawn_named("test-agent", "ping".to_string(), principal, None)
        .await
        .expect("spawn agent");

    // Subscribe to the session event bus immediately after spawn (before the
    // agent loop has reached the LLM call) and await `TurnMetrics`. This is
    // the event-driven replacement for the deadline+sleep poll: wakeup is
    // driven by the kernel emitting the turn-complete event; the timeout is
    // a safety bound that should never fire on a healthy turn.
    tk.watch_turn(session_key)
        .wait(Duration::from_secs(30))
        .await
        .expect("turn metrics");

    // Read the tape directly via the kernel's exposed TapeService.
    let tape_name = session_key.to_string();
    let entries = tk
        .handle
        .tape()
        .entries(&tape_name)
        .await
        .expect("read tape entries");

    // 1. No Message row may carry whitespace-only `content`. This is the primary
    //    regression assertion for issue #1979 — before the fix the iteration-1
    //    write produced exactly `content: "\n"`.
    let whitespace_messages: Vec<_> = entries
        .iter()
        .filter(|e| e.kind == TapEntryKind::Message)
        .filter(|e| {
            e.payload
                .get("content")
                .and_then(|c| c.as_str())
                .map(|s| !s.is_empty() && s.trim().is_empty())
                .unwrap_or(false)
        })
        .collect();
    assert!(
        whitespace_messages.is_empty(),
        "expected no whitespace-only assistant Message rows, found: {:#?}",
        whitespace_messages
            .iter()
            .map(|e| (e.id, e.payload.clone()))
            .collect::<Vec<_>>()
    );

    // 2. The cascade-tick boundary survives: the iteration-1 ToolCall row is still
    //    present.
    let tool_call_rows: Vec<_> = entries
        .iter()
        .filter(|e| e.kind == TapEntryKind::ToolCall)
        .collect();
    assert!(
        !tool_call_rows.is_empty(),
        "expected at least one ToolCall row to preserve the cascade tick boundary that PR 608 \
         introduced; found entries: {:#?}",
        entries.iter().map(|e| (e.id, e.kind)).collect::<Vec<_>>()
    );

    // 3. When iteration 1 is suppressed but reasoning was non-empty, the spec
    //    requires the row to be persisted with canonical empty content (`""`), not
    //    skipped. Find the assistant Message with matching `reasoning_content` and
    //    assert `content == ""`.
    let reasoning_rows: Vec<_> = entries
        .iter()
        .filter(|e| e.kind == TapEntryKind::Message)
        .filter(|e| {
            e.payload.get("reasoning_content").and_then(|v| v.as_str())
                == Some("internal chain of thought")
        })
        .collect();
    assert_eq!(
        reasoning_rows.len(),
        1,
        "expected exactly one assistant Message row carrying the scripted reasoning content; \
         entries: {:#?}",
        entries
            .iter()
            .map(|e| (e.id, e.kind, e.payload.clone()))
            .collect::<Vec<_>>()
    );
    let canonical_content = reasoning_rows[0]
        .payload
        .get("content")
        .and_then(|c| c.as_str())
        .expect("reasoning row carries a content field");
    assert_eq!(
        canonical_content, "",
        "whitespace content with non-empty reasoning must canonicalize to empty string, got: \
         {canonical_content:?}"
    );

    tk.shutdown();
}
