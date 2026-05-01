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

//! Lane-2 scripted e2e: LLM-prompt-context compression via [`ContextFolder`].
//!
//! Backstop for the "context compression" concern that the deleted
//! `real_tape_flow.rs` soak was nominally probing.  Drives
//! [`rara_kernel::agent::fold::ContextFolder`] (the only LLM-prompt-context
//! compressor in the kernel — see `kernel/src/agent/fold.rs`) with a
//! [`ScriptedLlmDriver`] so the test is deterministic, network-free, and runs
//! in milliseconds.
//!
//! What this test guards:
//!
//! 1. A long trajectory (120 messages mixing user / assistant / tool_result,
//!    over 25 KB of text) is reduced to a small `FoldSummary` whose serialized
//!    length is bounded — i.e. compression actually compresses.
//! 2. The fold call's outgoing `max_tokens` is clamped to
//!    `(source_token_estimate / 10).clamp(256, 2048)`, regardless of source
//!    size — guards the runaway-budget bug.
//! 3. Compounding: a second fold with `prior_summary` includes the prior in its
//!    prompt, so a long-running session can keep folding without quadratic
//!    growth.
//!
//! What this test does NOT cover (and rara has no other mechanism for, so
//! there is no kernel-internal target to test):
//!
//! - Trajectory truncation by message-count or token-budget *before* the LLM
//!   call (no such pre-trim path exists in `agent::runner` or
//!   `memory::context::build_llm_context` today; the entire compaction path is
//!   `ContextFolder`).  If a per-turn pre-trim is added later, add a peer test
//!   here.

use std::sync::Arc;

use rara_kernel::{
    agent::fold::ContextFolder,
    llm::{
        CompletionResponse, Message, MessageContent, ScriptedLlmDriver, StopReason, ToolCallRequest,
    },
};
use serde_json::json;

/// Build a long, realistic trajectory: alternating user / assistant /
/// tool_result entries, with enough bulk to exceed any reasonable
/// per-turn prompt budget.
fn long_trajectory(n_turns: usize) -> Vec<Message> {
    let mut out = Vec::with_capacity(n_turns * 3 + 1);
    out.push(Message::system(
        "You are a coding agent. Help the user investigate a long-running multi-tool debugging \
         session.",
    ));
    for i in 0..n_turns {
        out.push(Message::user(format!(
            "Turn {i}: Could you check what's happening in the kernel module after the last run? \
             Focus on memory pressure indicators."
        )));
        let tc = ToolCallRequest {
            id:        format!("call_{i}"),
            name:      "grep".to_string(),
            arguments: json!({"pattern": "memory_pressure", "path": "crates/kernel/src"})
                .to_string(),
        };
        out.push(Message::assistant_with_tool_calls(
            format!("I'll grep for memory_pressure markers in turn {i}."),
            vec![tc],
        ));
        out.push(Message::tool_result(
            format!("call_{i}"),
            format!(
                "match: crates/kernel/src/memory/store.rs:42: fn check_memory_pressure() {{ ... \
                 turn={i} ... }}\nmatch: crates/kernel/src/memory/context.rs:88: // pressure \
                 recomputed each turn ({i})\nmatch: crates/kernel/src/agent/runner.rs:201: // \
                 fold trigger threshold checked here, turn {i}"
            ),
        ));
    }
    out
}

fn fold_response(summary: &str, next_steps: &str) -> CompletionResponse {
    let body = json!({
        "summary": summary,
        "next_steps": next_steps,
    })
    .to_string();
    CompletionResponse {
        content:           Some(body),
        reasoning_content: None,
        tool_calls:        vec![],
        stop_reason:       StopReason::Stop,
        usage:             None,
        model:             "scripted-fold".to_string(),
    }
}

/// Rough char count of a [`Message`] (text content + tool-call args).
fn approx_len(msg: &Message) -> usize {
    let body = match &msg.content {
        MessageContent::Text(s) => s.len(),
        MessageContent::Multimodal(_) => 0,
    };
    let tcs: usize = msg
        .tool_calls
        .iter()
        .map(|tc| tc.name.len() + tc.arguments.to_string().len())
        .sum();
    body + tcs
}

#[tokio::test]
async fn fold_compresses_long_trajectory() {
    // 120 turns × 3 entries + 1 system = 361 messages, ~30 KB of text.
    let messages = long_trajectory(120);
    let total_chars: usize = messages.iter().map(approx_len).sum();
    assert!(
        total_chars > 25_000,
        "trajectory should be substantial, got {total_chars} chars",
    );

    // Source token estimate ~ chars / 4. With 120 turns that's ~7500 tokens,
    // so max_tokens = (7500/10).clamp(256, 2048) = 750.
    let source_tokens = total_chars / 4;

    let scripted_summary = "User asked across many turns about kernel memory pressure; agent ran \
                            grep repeatedly and found three relevant call sites.";
    let scripted_next = "Inspect store.rs:42 and runner.rs:201 next turn.";
    let driver = Arc::new(ScriptedLlmDriver::new(vec![fold_response(
        scripted_summary,
        scripted_next,
    )]));

    let folder = ContextFolder::new(driver.clone(), "scripted-fold".to_string());

    let summary = folder
        .fold_with_prior(None, &messages, source_tokens)
        .await
        .expect("fold should succeed");

    // 1. Compression: output is dramatically smaller than input.
    let summary_bytes = summary.summary.len() + summary.next_steps.len();
    assert!(
        summary_bytes < total_chars / 10,
        "fold output ({summary_bytes} chars) should be <10% of source ({total_chars} chars)",
    );
    assert_eq!(summary.summary, scripted_summary);
    assert_eq!(summary.next_steps, scripted_next);

    // 2. Budget clamping: max_tokens follows the documented formula.
    let captured = driver.captured_requests();
    assert_eq!(captured.len(), 1, "expected exactly one fold call");
    let req = &captured[0];
    let expected_max = (source_tokens / 10).clamp(256, 2048) as u32;
    assert_eq!(
        req.max_tokens,
        Some(expected_max),
        "max_tokens should be clamped per ContextFolder formula",
    );
    assert!(
        (256..=2048).contains(&req.max_tokens.unwrap()),
        "max_tokens must stay within [256, 2048] regardless of source size",
    );

    // 3. The fold prompt actually contains the trajectory content (so we're really
    //    compressing the messages, not folding empty input).
    let user_prompt = match &req.messages.last().expect("user prompt").content {
        MessageContent::Text(s) => s.clone(),
        MessageContent::Multimodal(_) => panic!("fold prompt should be plain text"),
    };
    assert!(
        user_prompt.contains("memory_pressure"),
        "fold prompt should embed the trajectory",
    );
    assert!(
        user_prompt.contains("Target output length"),
        "fold prompt should include the budget directive",
    );
}

#[tokio::test]
async fn fold_with_prior_summary_compounds() {
    // Simulate a session that has already been folded once: a second fold
    // call must include the prior summary in its prompt so the agent loop
    // can keep folding indefinitely without losing earlier context.
    let messages = long_trajectory(40);
    let prior = "Earlier: user investigated memory pressure across turns 0–119; found three call \
                 sites; next steps recorded.";

    let driver = Arc::new(ScriptedLlmDriver::new(vec![fold_response(
        "Combined summary of prior + new turns.",
        "Continue with store.rs inspection.",
    )]));
    let folder = ContextFolder::new(driver.clone(), "scripted-fold".to_string());

    let _summary = folder
        .fold_with_prior(Some(prior), &messages, 4_000)
        .await
        .expect("fold should succeed");

    let captured = driver.captured_requests();
    let req = &captured[0];
    let user_prompt = match &req.messages.last().expect("user prompt").content {
        MessageContent::Text(s) => s.clone(),
        MessageContent::Multimodal(_) => panic!("fold prompt should be plain text"),
    };
    assert!(
        user_prompt.contains(prior),
        "compounding fold must embed the prior summary in the prompt",
    );
    assert!(
        user_prompt.contains("Prior conversation history"),
        "compounding fold must mark the prior section explicitly",
    );
}

#[tokio::test]
async fn fold_max_tokens_clamps_at_lower_bound_for_short_input() {
    // Tiny source — formula floor of 256 must hold so the LLM has room to
    // respond at all.
    let messages = vec![
        Message::system("system"),
        Message::user("one"),
        Message::assistant("two"),
    ];
    let driver = Arc::new(ScriptedLlmDriver::new(vec![fold_response("s", "n")]));
    let folder = ContextFolder::new(driver.clone(), "scripted-fold".to_string());

    let _ = folder
        .fold_with_prior(None, &messages, 50)
        .await
        .expect("fold should succeed");

    let captured = driver.captured_requests();
    assert_eq!(captured[0].max_tokens, Some(256));
}

#[tokio::test]
async fn fold_max_tokens_clamps_at_upper_bound_for_huge_input() {
    let messages = vec![Message::user("placeholder")];
    let driver = Arc::new(ScriptedLlmDriver::new(vec![fold_response("s", "n")]));
    let folder = ContextFolder::new(driver.clone(), "scripted-fold".to_string());

    // 1_000_000 tokens / 10 = 100_000, must clamp to 2048.
    let _ = folder
        .fold_with_prior(None, &messages, 1_000_000)
        .await
        .expect("fold should succeed");

    let captured = driver.captured_requests();
    assert_eq!(captured[0].max_tokens, Some(2048));
}
