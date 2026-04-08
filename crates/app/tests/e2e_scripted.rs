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

//! CI-ready E2E tests using [`ScriptedLlmDriver`] — no API keys, no
//! network, deterministic.
//!
//! Each test boots a minimal kernel via [`TestKernelBuilder`], sends
//! messages through the standard `KernelHandle` API, and asserts on turn
//! traces and tape entries.

use std::{path::Path, sync::Once, time::Duration};

use rara_kernel::{
    channel::types::{ChannelType, MessageContent},
    identity::{Principal, UserId},
    io::{ChannelSource, InboundMessage, MessageId},
    session::SessionKey,
    testing::{TestKernelBuilder, scripted_response},
};
use tokio::time::{Instant, sleep};

/// Override rara_paths directories to a writable temp path so tests
/// don't touch `~/.config/rara` (which may not exist on CI runners).
fn init_test_env(tmp: &Path) {
    static INIT: Once = Once::new();
    let data = tmp.join("rara_data");
    let config = tmp.join("rara_config");
    INIT.call_once(move || {
        rara_paths::set_custom_data_dir(&data);
        rara_paths::set_custom_config_dir(&config);
    });
}

/// Build an [`InboundMessage`] for test submission.
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
            platform_user_id:    "test".to_string(),
            platform_chat_id:    Some(chat_id.to_string()),
        },
        UserId("test".to_string()),
        session_key,
        None,
        MessageContent::Text(text.to_string()),
        None,
        jiff::Timestamp::now(),
        Default::default(),
    )
}

/// Poll until the session has at least `expected_turns` completed turns.
async fn wait_for_turn_count(
    handle: &rara_kernel::handle::KernelHandle,
    session_key: SessionKey,
    expected_turns: usize,
) {
    let deadline = Instant::now() + Duration::from_secs(10);
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
        sleep(Duration::from_millis(50)).await;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn simple_text_reply() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_env(tmp.path());
    let tk = TestKernelBuilder::new(tmp.path())
        .responses(vec![
            scripted_response("Hi there!"),
            // Padding for auxiliary LLM calls
            scripted_response("(padding)"),
        ])
        .build()
        .await;

    let principal = Principal::lookup("test".to_string());
    let session_key = tk
        .handle
        .spawn_named(&tk.agent_name, "hello".to_string(), principal, None)
        .await
        .expect("spawn session");

    wait_for_turn_count(&tk.handle, session_key, 1).await;

    let traces = tk.handle.get_process_turns(session_key);
    assert_eq!(traces.len(), 1, "should have exactly 1 turn");
    let turn = &traces[0];
    assert!(turn.success, "turn should succeed: {:?}", turn.error);

    // The last iteration should contain our scripted text.
    let preview = turn
        .iterations
        .last()
        .map(|i| i.text_preview.as_str())
        .unwrap_or("");
    assert!(
        preview.contains("Hi there!"),
        "expected scripted response in preview, got: {preview}"
    );

    tk.shutdown();
}

#[tokio::test]
async fn multi_turn_conversation() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_env(tmp.path());
    // Use a uniform response so the test is order-insensitive. The kernel
    // may make auxiliary LLM calls (knowledge extraction) between user
    // turns, consuming extra scripted responses.
    let tk = TestKernelBuilder::new(tmp.path())
        .responses(vec![
            scripted_response("scripted reply"),
            scripted_response("scripted reply"),
            scripted_response("scripted reply"),
            scripted_response("scripted reply"),
            scripted_response("scripted reply"),
            scripted_response("scripted reply"),
        ])
        .build()
        .await;

    let principal = Principal::lookup("test".to_string());
    let chat_id = "multi-turn-test";

    // Turn 1: spawn with initial input
    let session_key = tk
        .handle
        .spawn_named(&tk.agent_name, "First message".to_string(), principal, None)
        .await
        .expect("spawn session");

    wait_for_turn_count(&tk.handle, session_key, 1).await;

    // Turn 2: follow-up message
    tk.handle
        .submit_message(build_test_message(
            Some(session_key),
            chat_id,
            "Second message",
        ))
        .expect("submit turn 2");
    wait_for_turn_count(&tk.handle, session_key, 2).await;

    // Turn 3: another follow-up
    tk.handle
        .submit_message(build_test_message(
            Some(session_key),
            chat_id,
            "Third message",
        ))
        .expect("submit turn 3");
    wait_for_turn_count(&tk.handle, session_key, 3).await;

    let traces = tk.handle.get_process_turns(session_key);
    assert_eq!(traces.len(), 3, "should have 3 turns");

    // Verify each turn succeeded and produced output.
    for (i, turn) in traces.iter().enumerate() {
        assert!(
            turn.success,
            "turn {} should succeed: {:?}",
            i + 1,
            turn.error
        );
        let preview = turn
            .iterations
            .last()
            .map(|it| it.text_preview.as_str())
            .unwrap_or("");
        assert!(
            preview.contains("scripted reply"),
            "turn {} preview should contain scripted reply, got: {preview}",
            i + 1
        );
    }

    tk.shutdown();
}

#[tokio::test]
async fn empty_llm_response_handled() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_env(tmp.path());

    // Script an empty response (no content, no tool calls).
    let empty_response = rara_kernel::llm::CompletionResponse {
        content:           None,
        reasoning_content: None,
        tool_calls:        vec![],
        stop_reason:       rara_kernel::llm::StopReason::Stop,
        usage:             None,
        model:             "scripted".to_string(),
    };

    let tk = TestKernelBuilder::new(tmp.path())
        .responses(vec![
            empty_response,
            // Padding for auxiliary LLM calls
            scripted_response("(padding)"),
            scripted_response("(padding)"),
        ])
        .build()
        .await;

    let principal = Principal::lookup("test".to_string());
    let session_key = tk
        .handle
        .spawn_named(&tk.agent_name, "say something".to_string(), principal, None)
        .await
        .expect("spawn session");

    wait_for_turn_count(&tk.handle, session_key, 1).await;

    let traces = tk.handle.get_process_turns(session_key);
    assert_eq!(traces.len(), 1);

    // The turn should still complete (success or graceful handling).
    // An empty response is a valid LLM output.
    let turn = &traces[0];
    assert!(
        turn.success,
        "empty response should not crash the session: {:?}",
        turn.error
    );

    tk.shutdown();
}

#[tokio::test]
async fn tape_records_conversation() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_env(tmp.path());
    let tk = TestKernelBuilder::new(tmp.path())
        .responses(vec![
            scripted_response("Recorded reply"),
            // Padding for auxiliary LLM calls
            scripted_response("(padding)"),
        ])
        .build()
        .await;

    let principal = Principal::lookup("test".to_string());
    let session_key = tk
        .handle
        .spawn_named(&tk.agent_name, "tape test".to_string(), principal, None)
        .await
        .expect("spawn session");

    wait_for_turn_count(&tk.handle, session_key, 1).await;

    // Read tape entries for this session.
    let tape = tk.handle.tape();
    let tape_name = session_key.to_string();
    let entries = tape
        .entries(&tape_name)
        .await
        .expect("tape entries should load");

    // There should be at least some entries (session start, user message,
    // assistant response).
    assert!(
        !entries.is_empty(),
        "tape should have recorded entries for the session"
    );

    tk.shutdown();
}
