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

//! Mock-provider driver-stack e2e (issue #2190).
//!
//! Boots the full app via `start_with_options()` and drives multi-turn
//! conversations through the REAL openai driver (HTTP + auth + SSE parsing in
//! `crates/kernel/src/llm/{openai,stream}.rs`) against an in-process
//! wiremock OpenAI fake. Deterministic, zero-cost, secret-free — replaces the
//! retired real-LLM CI lane (#1941 → #2178 → #2190).
//!
//! The test is hermetic: it writes its own `config.yaml` under a temp
//! `rara_paths` custom data dir (the ONLY reliable injection point for the
//! mock's `base_url` — `ConfigFileSync` seeds the settings store from the
//! config FILE at boot, so mutating the in-memory `AppConfig` never reaches
//! the driver's per-request settings lookup).

mod common;

use std::time::Duration;

use common::mock_provider::{
    self, finish_chunk, reasoning_chunk, text_chunk, tool_call_args_chunk, tool_call_start_chunk,
};
use rara_app::{AppConfig, StartOptions, start_with_options};
use rara_kernel::{
    channel::types::{ChannelType, MessageContent},
    identity::{Principal, UserId},
    io::{ChannelSource, InboundMessage, MessageId},
    memory::{FileTapeStore, TapEntryKind, TapeService},
    session::SessionKey,
};
use serde_json::Value;
use tokio::time::{Instant, sleep};
use wiremock::MockServer;

const TURN_TIMEOUT_SECS: u64 = 60;

/// Unique content marker inside the workspace note file the scripted tool
/// call reads — proves real tool output flowed back into the follow-up LLM
/// request.
const NOTE_MARKER: &str = "MOCK-NOTE-MARKER-7391";

const TURN1_TEXT: &str = "记住这个数字：42是宇宙的答案。确认你记住了。";
const TURN2_TEXT: &str =
    "现在我们讨论一个新话题：Rust的所有权系统。简短解释下 borrow checker 的核心规则。";
const TURN3_TEXT: &str = "42是什么的答案？只回答数字和含义。";

fn build_test_message(
    session_key: Option<SessionKey>,
    chat_id: &str,
    text: &str,
) -> InboundMessage {
    InboundMessage::unresolved(
        MessageId::new(),
        ChannelSource {
            channel_type:        ChannelType::Internal,
            platform_message_id: None,
            platform_user_id:    "ryan".to_string(),
            platform_chat_id:    Some(chat_id.to_string()),
        },
        UserId("ryan".to_string()),
        session_key,
        None,
        MessageContent::Text(text.to_string()),
        None,
        jiff::Timestamp::now(),
        Default::default(),
    )
}

async fn wait_for_turn_count(
    handle: &rara_kernel::handle::KernelHandle,
    session_key: SessionKey,
    expected_turns: usize,
) {
    let deadline = Instant::now() + Duration::from_secs(TURN_TIMEOUT_SECS);
    loop {
        let traces = handle.get_process_turns(session_key);
        if traces.len() >= expected_turns {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for turn {expected_turns} in session {session_key}; \
             current_turns={} latest_trace={:?}",
            traces.len(),
            traces.last()
        );
        sleep(Duration::from_millis(500)).await;
    }
}

/// Write the hermetic test config pointing the openai provider at the mock
/// server. Required sections mirror `ci/config.template.yaml`.
fn write_test_config(config_path: &std::path::Path, mock_base_url: &str) {
    let yaml = format!(
        r#"http:
  bind_address: "127.0.0.1:0"
  cors_allowed_origins:
    - "http://localhost:5173"

grpc:
  bind_address: "127.0.0.1:0"
  server_address: "127.0.0.1:0"

owner_token: "e2e-mock-owner-token-not-a-real-secret"
owner_user_id: "ryan"

users:
  - name: "ryan"
    role: root
    platforms: []

mita:
  heartbeat_interval: "30m"

llm:
  default_provider: "openai"
  providers:
    openai:
      base_url: "{mock_base_url}"
      api_key: "mock-key-not-a-real-secret"
      default_model: "mock-model"

knowledge:
  embedding_model: "mock-embedding"
  embedding_dimensions: {dims}
  search_top_k: 5
  similarity_threshold: 0.85
"#,
        dims = mock_provider::EMBEDDING_DIMENSIONS,
    );
    std::fs::create_dir_all(
        config_path
            .parent()
            .expect("config path should have a parent"),
    )
    .expect("config dir should be creatable");
    std::fs::write(config_path, yaml).expect("test config should be writable");
}

/// Script the conversation on the mock server.
///
/// Priorities: later turns get numerically lower (= higher) priority because
/// each turn's request embeds all earlier turns' text in its message history.
async fn mount_conversation(server: &MockServer, note_path: &str) {
    // Turn 3 (recall): CJK multi-chunk answer containing the "42" fact.
    mock_provider::mount_chat_sse(
        server,
        TURN3_TEXT,
        1,
        &[
            text_chunk("42是宇宙、生命以及"),
            text_chunk("一切问题的终极答案。"),
            finish_chunk("stop"),
        ],
    )
    .await;

    // Turn 2 follow-up (after the read-file tool round): matched on the tool
    // output marker, which only appears once the tool result is in the
    // request history.
    mock_provider::mount_chat_sse(
        server,
        NOTE_MARKER,
        2,
        &[
            text_chunk("borrow checker 的核心规则："),
            text_chunk("同一时刻要么一个可变引用，要么任意多个不可变引用。"),
            finish_chunk("stop"),
        ],
    )
    .await;

    // Turn 2 (first round): a scripted tool call to `read-file`, with the
    // JSON arguments split across two SSE chunks to exercise
    // `ToolCallArgumentsDelta` accumulation in the real driver.
    let args = serde_json::json!({ "file_path": note_path }).to_string();
    let (args_head, args_tail) = args.split_at(args.len() / 2);
    mock_provider::mount_chat_sse(
        server,
        TURN2_TEXT,
        3,
        &[
            tool_call_start_chunk("call-mock-1", "read-file"),
            tool_call_args_chunk(args_head),
            tool_call_args_chunk(args_tail),
            finish_chunk("tool_calls"),
        ],
    )
    .await;

    // Turn 1: multi-chunk CJK text + a reasoning delta, streamed through the
    // real SSE parser.
    mock_provider::mount_chat_sse(
        server,
        "记住这个数字",
        4,
        &[
            reasoning_chunk("用户要求记住一个事实。"),
            text_chunk("好的，我已经记住了："),
            text_chunk("42是宇宙的答案。"),
            finish_chunk("stop"),
        ],
    )
    .await;

    // Background traffic (title_gen, knowledge_extractor, embeddings).
    mock_provider::mount_fallbacks(server).await;
}

#[tokio::test]
#[ignore = "full-app-boot e2e (mock provider, no secrets) — runs via e2e.yml; locally: cargo test \
            -p rara-app --test anchor_checkout_e2e -- --ignored --nocapture"]
async fn anchor_checkout_roundtrip() {
    // 1. Setup — hermetic paths, mock provider, test-authored config.
    common_telemetry::logging::init_default_ut_logging();

    let data_dir = tempfile::tempdir().expect("temp data dir should be creatable");
    let workspace = tempfile::tempdir().expect("temp workspace should be creatable");
    // Must run before anything touches rara_paths: steers data_dir AND
    // config_dir (=> `config_file()`) under the temp dir.
    rara_paths::set_custom_data_dir(data_dir.path());
    std::env::set_current_dir(workspace.path()).expect("should switch to temp workspace");

    let note_path = workspace.path().join("mock-note.txt");
    std::fs::write(&note_path, format!("{NOTE_MARKER}: 关于42的补充说明。\n"))
        .expect("note file should be writable");

    let server = MockServer::start().await;
    mount_conversation(&server, note_path.to_str().expect("utf8 temp path")).await;
    write_test_config(rara_paths::config_file(), &server.uri());

    let mut config = AppConfig::new().expect("config should load");
    // Defend against a portless-injected PORT env rewriting the bind address.
    config.http.bind_address = "127.0.0.1:0".to_string();

    let mut app = start_with_options(config, StartOptions::default())
        .await
        .expect("app should start");
    let handle = app
        .kernel_handle
        .take()
        .expect("kernel handle should be available");

    // 2. Create session with a verifiable fact
    let principal = Principal::lookup("ryan".to_string());
    let session_key = handle
        .spawn_named("rara", TURN1_TEXT.to_string(), principal.clone(), None)
        .await
        .expect("should spawn session");

    let chat_id = format!("e2e-checkout-{}", uuid::Uuid::new_v4());
    wait_for_turn_count(&handle, session_key, 1).await;

    // Verify turn 1 succeeded — print the trace on failure so a failed
    // turn's provider error (carried since #2178) is visible verbatim.
    let traces = handle.get_process_turns(session_key);
    let turn1 = traces.last().unwrap();
    assert!(
        turn1.success,
        "turn 1 should succeed; latest_trace={turn1:?}"
    );
    // The multi-chunk CJK SSE stream must be reassembled verbatim by the
    // real driver (stream_chat_completions + StreamAccumulator).
    let turn1_preview = turn1
        .iterations
        .last()
        .map(|i| i.text_preview.clone())
        .unwrap_or_default();
    assert!(
        turn1_preview.contains("42是宇宙的答案"),
        "turn 1 preview should carry the scripted CJK content, got: {turn1_preview}"
    );

    // 3. Send another message to build context — scripted as a real tool
    // round: the mock returns a `read-file` tool call, the app executes it,
    // and the follow-up request carries the tool result back to the mock.
    handle
        .submit_message(build_test_message(Some(session_key), &chat_id, TURN2_TEXT))
        .expect("msg 2 should submit");
    wait_for_turn_count(&handle, session_key, 2).await;

    let traces = handle.get_process_turns(session_key);
    let turn2 = traces.last().unwrap();
    assert!(
        turn2.success,
        "turn 2 should succeed; latest_trace={turn2:?}"
    );
    assert_eq!(
        turn2.total_tool_calls, 1,
        "turn 2 should execute exactly the scripted read-file call; trace={turn2:?}"
    );
    assert_eq!(
        turn2.iterations.len(),
        2,
        "turn 2 should take two LLM iterations (tool call + final answer); trace={turn2:?}"
    );

    // 4. Verify tape has entries and find anchors
    let tape_service = TapeService::new(
        FileTapeStore::new(rara_paths::memory_dir(), workspace.path())
            .await
            .expect("tape store should open"),
    );
    let session_tape = session_key.to_string();
    let entries_before = tape_service
        .entries(&session_tape)
        .await
        .expect("entries should load");
    let entry_count_before = entries_before.len();
    eprintln!("entries before checkout: {entry_count_before}");

    // Find anchors
    let anchors = tape_service
        .anchors(&session_tape, 10)
        .await
        .expect("anchors should load");
    eprintln!("anchors found: {}", anchors.len());
    assert!(
        !anchors.is_empty(),
        "should have at least session/start anchor"
    );

    // Use the first anchor for checkout
    let anchor_name = &anchors[0].name;
    eprintln!("will checkout from anchor: {anchor_name}");

    // 5. Checkout — create a fork at the anchor
    let new_session_tape = format!("{session_tape}__e2e_checkout");
    tape_service
        .checkout_anchor(&session_tape, anchor_name, &new_session_tape)
        .await
        .expect("checkout should succeed");

    // 6. Verify fork tape
    let fork_entries = tape_service
        .entries(&new_session_tape)
        .await
        .expect("fork entries should load");
    eprintln!(
        "fork entries: {}, original entries: {}",
        fork_entries.len(),
        entry_count_before
    );

    // Fork should have fewer or equal entries (up to anchor only)
    assert!(
        fork_entries.len() <= entry_count_before,
        "fork should not have more entries than original"
    );

    // Fork should contain the anchor
    assert!(
        fork_entries.iter().any(|e| {
            e.kind == TapEntryKind::Anchor
                && e.payload
                    .get("name")
                    .and_then(|v| v.as_str())
                    .is_some_and(|n| n == anchor_name)
        }),
        "fork should contain the checkout anchor"
    );

    // 7. Append to fork — should not affect parent
    tape_service
        .append_message(
            &new_session_tape,
            serde_json::json!({"role": "user", "content": "this is in the fork only"}),
            None,
        )
        .await
        .expect("append to fork should succeed");

    // Parent tape should be unchanged
    let parent_entries_after = tape_service
        .entries(&session_tape)
        .await
        .expect("parent entries should load");
    assert_eq!(
        parent_entries_after.len(),
        entry_count_before,
        "parent tape should not be modified by fork operations"
    );

    // Fork should have the new message
    let fork_entries_after = tape_service
        .entries(&new_session_tape)
        .await
        .expect("fork entries after append should load");
    assert!(
        fork_entries_after.iter().any(|e| {
            e.payload
                .get("content")
                .and_then(|v| v.as_str())
                .is_some_and(|c| c == "this is in the fork only")
        }),
        "fork should contain the appended message"
    );
    assert!(
        !parent_entries_after.iter().any(|e| {
            e.payload
                .get("content")
                .and_then(|v| v.as_str())
                .is_some_and(|c| c == "this is in the fork only")
        }),
        "parent should NOT contain the fork's message"
    );

    // 8. Continue conversation in original session — should still work
    handle
        .submit_message(build_test_message(Some(session_key), &chat_id, TURN3_TEXT))
        .expect("recall msg should submit");
    wait_for_turn_count(&handle, session_key, 3).await;

    let traces = handle.get_process_turns(session_key);
    let recall_trace = traces.last().unwrap();
    assert!(
        recall_trace.success,
        "recall turn should succeed; latest_trace={recall_trace:?}"
    );
    let preview = recall_trace
        .iterations
        .last()
        .map(|i| i.text_preview.clone())
        .unwrap_or_default();
    eprintln!("recall response: {preview}");
    // Scripted response — rara-owned assertion (the #1941 model-recall
    // `contains("42")` assertion is retired; the mock always answers with
    // the fact).
    assert!(
        preview.contains("42是宇宙"),
        "recall preview should carry the scripted answer, got: {preview}"
    );

    // 9. Captured-request assertions — the deterministic replacement for the
    // old model-behavior check: context assembly through the full app stack
    // + real driver serialization is what actually recalls "42".
    let requests = mock_provider::chat_request_bodies(&server).await;
    assert_request_context(&requests);

    eprintln!("E2E anchor checkout test passed!");
    app.shutdown();
}

/// Post-hoc assertions on the requests the real driver actually sent.
fn assert_request_context(requests: &[Value]) {
    // Agent-turn requests are identified structurally: they carry a user
    // message byte-equal to the turn's input text (see
    // `mock_provider::user_message_index` for why equality stays unique).
    let (recall_request, turn3_index) = requests
        .iter()
        .rev()
        .find_map(|r| mock_provider::user_message_index(r, TURN3_TEXT).map(|i| (r, i)))
        .unwrap_or_else(|| {
            panic!(
                "no captured request carries {TURN3_TEXT:?} as a user message; captured {} chat \
                 requests",
                requests.len()
            )
        });

    // The driver must have requested streaming — proves the SSE path
    // (`stream_chat_completions`) is what parsed the mock's responses.
    assert_eq!(
        recall_request.get("stream"),
        Some(&Value::Bool(true)),
        "recall request should be a streaming request"
    );

    // Context assembly: the recall request must carry the "42" fact from
    // turn 1 of the same session, EARLIER in the history than the recall
    // question itself (the question does not contain the fact, so this
    // cannot pass vacuously).
    let messages = recall_request
        .get("messages")
        .and_then(Value::as_array)
        .expect("recall request should have messages");
    let history_has_fact = messages[..turn3_index].iter().any(|m| {
        m.get("content")
            .and_then(Value::as_str)
            .is_some_and(|c| c.contains("42是宇宙的答案"))
    });
    assert!(
        history_has_fact,
        "recall request history should contain the 42 fact from turn 1; messages={messages:?}"
    );

    // Tool round: the follow-up request after the scripted read-file call
    // must carry the tool result (role:"tool", matching id, real file
    // content) — proves the full tool loop ran over the real wire format.
    let tool_followup = requests
        .iter()
        .find(|r| {
            r.get("messages")
                .and_then(Value::as_array)
                .is_some_and(|msgs| {
                    msgs.iter().any(|m| {
                        m.get("role").and_then(Value::as_str) == Some("tool")
                            && m.get("tool_call_id").and_then(Value::as_str) == Some("call-mock-1")
                    })
                })
        })
        .expect("a captured request should carry the read-file tool result");
    let tool_result_has_marker = tool_followup
        .get("messages")
        .and_then(Value::as_array)
        .expect("tool follow-up should have messages")
        .iter()
        .filter(|m| m.get("role").and_then(Value::as_str) == Some("tool"))
        .any(|m| {
            m.get("content")
                .and_then(Value::as_str)
                .is_some_and(|c| c.contains(NOTE_MARKER))
        });
    assert!(
        tool_result_has_marker,
        "tool result content should contain the note marker {NOTE_MARKER}; \
         request={tool_followup:?}"
    );
}
