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

use std::{
    path::PathBuf,
    sync::{Arc, Once},
    time::Duration,
};

use rara_kernel::{
    KernelError,
    channel::types::{ChannelType, MessageContent},
    identity::{Principal, UserId},
    io::{ChannelSource, InboundMessage, MessageId},
    llm::{CompletionResponse, StopReason, ToolCallRequest},
    session::{SessionKey, SessionState},
    testing::{FakeTool, TestKernelBuilder, scripted_response, scripted_tool_call_response},
};
use serde_json::json;
use tokio::time::{Instant, sleep};

/// CI runners can be noisy under full-workspace `nextest`; keep a generous
/// upper bound for end-to-end turn completion.
const TURN_WAIT_TIMEOUT: Duration = Duration::from_secs(30);

/// Override rara_paths directories to a writable temp path so tests
/// don't touch `~/.config/rara` (which may not exist on CI runners).
fn init_test_env() {
    static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    static INIT: Once = Once::new();
    let root = ROOT.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("rara-test-env-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create stable test env root");
        dir
    });
    INIT.call_once(move || {
        let data = root.join("rara_data");
        let config = root.join("rara_config");
        std::fs::create_dir_all(&data).expect("create stable test data dir");
        std::fs::create_dir_all(&config).expect("create stable test config dir");
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
    let deadline = Instant::now() + TURN_WAIT_TIMEOUT;
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

/// Wait for a session to complete its current turn and return to Ready.
///
/// `handle_spawn_agent` pushes a UserMessage to the event queue and
/// returns — the session is still Ready when `spawn_named` completes.
/// We poll until `messages_received >= 1` (the turn started, incrementing
/// the counter in `start_llm_turn`) AND state is Ready (the turn finished).
/// This distinguishes pre-turn Ready from post-turn Ready without needing
/// to observe the brief Active window.
async fn wait_for_session_ready(
    handle: &rara_kernel::handle::KernelHandle,
    session_key: SessionKey,
) {
    let deadline = Instant::now() + TURN_WAIT_TIMEOUT;
    loop {
        if let Some(stats) = handle.session_stats(session_key) {
            if stats.messages_received >= 1 && matches!(stats.state, SessionState::Ready) {
                return;
            }
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for session {session_key} to return to Ready after processing \
             initial message"
        );
        sleep(Duration::from_millis(5)).await;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn simple_text_reply() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_env();
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn multi_turn_conversation() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_env();
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn empty_llm_response_handled() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_env();

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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tool_call_round_trip() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_env();

    // The FakeTool echoes back a single scripted result.
    let fake_tool = Arc::new(FakeTool::new(
        "echo",
        vec![json!({"output": "hello world"})],
    ));

    // Script: turn 1 asks the LLM, which requests the `echo` tool; after the
    // tool result is fed back, the LLM produces the final user-visible reply.
    // Extra padding covers any auxiliary calls (e.g. knowledge extraction).
    let tk = TestKernelBuilder::new(tmp.path())
        .with_tool(fake_tool.clone())
        .responses(vec![
            scripted_tool_call_response(vec![ToolCallRequest {
                id:        "call_echo_1".to_string(),
                name:      "echo".to_string(),
                arguments: json!({"text": "hello"}).to_string(),
            }]),
            scripted_response("The tool said: hello world"),
            scripted_response("(padding)"),
            scripted_response("(padding)"),
        ])
        .build()
        .await;

    let principal = Principal::lookup("test".to_string());
    let session_key = tk
        .handle
        .spawn_named(
            &tk.agent_name,
            "use the echo tool".to_string(),
            principal,
            None,
        )
        .await
        .expect("spawn session");

    wait_for_turn_count(&tk.handle, session_key, 1).await;

    // The tool must have been invoked exactly once with the scripted args.
    let inputs = fake_tool.captured_inputs();
    assert_eq!(
        inputs.len(),
        1,
        "FakeTool should be called exactly once, got: {inputs:?}"
    );
    assert_eq!(
        inputs[0],
        json!({"text": "hello"}),
        "FakeTool received unexpected arguments"
    );

    // The final iteration should carry the LLM's post-tool reply.
    let traces = tk.handle.get_process_turns(session_key);
    assert_eq!(traces.len(), 1, "should have exactly 1 turn");
    let turn = &traces[0];
    assert!(turn.success, "turn should succeed: {:?}", turn.error);
    assert!(
        turn.iterations.len() >= 2,
        "expected at least 2 iterations (tool call + final reply), got {}",
        turn.iterations.len()
    );
    let preview = turn
        .iterations
        .last()
        .map(|i| i.text_preview.as_str())
        .unwrap_or("");
    assert!(
        preview.contains("hello world"),
        "expected tool output to surface in final reply, got: {preview}"
    );

    tk.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tape_records_conversation() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_env();
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

// ---------------------------------------------------------------------------
// Failure-mode tests (#1179)
// ---------------------------------------------------------------------------

/// LLM returns a non-retryable error on the first call. The session should
/// handle the error and return to Ready state — not crash or hang.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn llm_error_does_not_crash_session() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_env();

    // First call: non-retryable provider error. The agent loop surfaces this
    // as an AgentExecution error (no TurnTrace is pushed for hard errors).
    // Subsequent calls: normal responses so a follow-up turn can succeed.
    let tk = TestKernelBuilder::new(tmp.path())
        .with_results(vec![
            Err(KernelError::NonRetryable {
                message: "simulated provider failure".into(),
            }),
            Ok(scripted_response("recovered")),
            Ok(scripted_response("(padding)")),
            Ok(scripted_response("(padding)")),
        ])
        .build()
        .await;

    let principal = Principal::lookup("test".to_string());
    let session_key = tk
        .handle
        .spawn_named(&tk.agent_name, "trigger error".to_string(), principal, None)
        .await
        .expect("spawn session");

    // The agent loop returns Err for non-retryable errors, so no TurnTrace
    // is pushed. Wait for the error turn to complete (session returns to
    // Ready) before sending the follow-up, otherwise the follow-up lands
    // while the first turn is Active and triggers the interrupt flag.
    wait_for_session_ready(&tk.handle, session_key).await;

    // Session is alive and Ready — send a second message to prove it
    // did not crash.
    let chat_id = "error-recovery-test";
    tk.handle
        .submit_message(build_test_message(
            Some(session_key),
            chat_id,
            "Are you still there?",
        ))
        .expect("submit follow-up message");

    wait_for_turn_count(&tk.handle, session_key, 1).await;

    let traces = tk.handle.get_process_turns(session_key);
    assert_eq!(traces.len(), 1, "follow-up turn should produce a trace");
    assert!(
        traces[0].success,
        "follow-up turn should succeed: {:?}",
        traces[0].error
    );

    tk.shutdown();
}

/// With max_iterations=3 in the default test manifest, scripting infinite
/// tool calls should terminate after 3 iterations rather than looping forever.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn max_iterations_terminates() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_env();

    let fake_tool = Arc::new(FakeTool::new(
        "loopy",
        vec![
            json!({"status": "ok"}),
            json!({"status": "ok"}),
            json!({"status": "ok"}),
        ],
    ));

    // Every LLM response requests another tool call. The manifest's
    // max_iterations=3 should terminate the loop.
    let tool_call = |id: &str| {
        scripted_tool_call_response(vec![ToolCallRequest {
            id:        id.to_string(),
            name:      "loopy".to_string(),
            arguments: json!({"x": 1}).to_string(),
        }])
    };
    let tk = TestKernelBuilder::new(tmp.path())
        .with_tool(fake_tool.clone())
        .responses(vec![
            tool_call("c1"),
            tool_call("c2"),
            tool_call("c3"),
            // Extra padding in case the loop consumes more.
            scripted_response("(overflow)"),
            scripted_response("(overflow)"),
        ])
        .build()
        .await;

    let principal = Principal::lookup("test".to_string());
    let session_key = tk
        .handle
        .spawn_named(&tk.agent_name, "loop forever".to_string(), principal, None)
        .await
        .expect("spawn session");

    wait_for_turn_count(&tk.handle, session_key, 1).await;

    let traces = tk.handle.get_process_turns(session_key);
    assert_eq!(traces.len(), 1, "should have exactly 1 turn");
    let turn = &traces[0];

    // Max iterations reached — the turn should be marked as failed with
    // an error mentioning "max iterations".
    assert!(
        !turn.success,
        "turn should fail when max iterations exhausted"
    );
    let err_msg = turn.error.as_deref().unwrap_or("");
    assert!(
        err_msg.contains("max iterations"),
        "error should mention max iterations, got: {err_msg}"
    );

    tk.shutdown();
}

/// LLM calls a tool that is not registered. The kernel should feed the
/// error back to the LLM, which then produces a text response on the
/// second call.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tool_not_found_surfaces_error() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_env();

    // First response: call a nonexistent tool.
    // Second response: normal text (the LLM "recovers" after seeing the error).
    let tk = TestKernelBuilder::new(tmp.path())
        .responses(vec![
            scripted_tool_call_response(vec![ToolCallRequest {
                id:        "call_ghost".to_string(),
                name:      "nonexistent_tool".to_string(),
                arguments: json!({"q": "hello"}).to_string(),
            }]),
            scripted_response("I could not find that tool, here is a direct answer."),
            scripted_response("(padding)"),
            scripted_response("(padding)"),
        ])
        .build()
        .await;

    let principal = Principal::lookup("test".to_string());
    let session_key = tk
        .handle
        .spawn_named(
            &tk.agent_name,
            "call missing tool".to_string(),
            principal,
            None,
        )
        .await
        .expect("spawn session");

    wait_for_turn_count(&tk.handle, session_key, 1).await;

    let traces = tk.handle.get_process_turns(session_key);
    assert_eq!(traces.len(), 1, "should have exactly 1 turn");
    let turn = &traces[0];

    // The turn should succeed — tool-not-found is fed back as a tool
    // result error, then the LLM produces a valid text response.
    assert!(turn.success, "turn should succeed: {:?}", turn.error);
    assert!(
        turn.iterations.len() >= 2,
        "expected at least 2 iterations (tool call + text reply), got {}",
        turn.iterations.len()
    );
    let preview = turn
        .iterations
        .last()
        .map(|i| i.text_preview.as_str())
        .unwrap_or("");
    assert!(
        preview.contains("direct answer"),
        "final reply should contain the scripted text, got: {preview}"
    );

    tk.shutdown();
}

/// Script several consecutive empty responses (no text, no tool calls).
/// The kernel's recovery logic should eventually terminate rather than
/// looping indefinitely.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn consecutive_empty_responses_terminate() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_env();

    let empty = || CompletionResponse {
        content:           None,
        reasoning_content: None,
        tool_calls:        vec![],
        stop_reason:       StopReason::Stop,
        usage:             None,
        model:             "scripted".to_string(),
    };

    // 5 consecutive empty responses. The kernel's MAX_LLM_ERROR_RECOVERIES
    // (3) + max_iterations (3) should bound the total iterations.
    let tk = TestKernelBuilder::new(tmp.path())
        .responses(vec![
            empty(),
            empty(),
            empty(),
            empty(),
            empty(),
            // Extra padding so the driver doesn't run out prematurely.
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

    // The turn must complete within the timeout — no infinite loop.
    wait_for_turn_count(&tk.handle, session_key, 1).await;

    let traces = tk.handle.get_process_turns(session_key);
    assert_eq!(traces.len(), 1, "should have exactly 1 turn");

    // We don't assert success/failure — the important property is that
    // the turn terminated rather than hanging.

    tk.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_file_delivers_attachment() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_env();

    // Create a temp file to send.
    let file_path = tmp.path().join("test.pdf");
    std::fs::write(&file_path, b"fake pdf content").expect("write test file");
    let file_path_str = file_path.to_str().expect("valid utf8 path");

    // Script: LLM calls send-file with the temp path, then produces a reply.
    let tk = TestKernelBuilder::new(tmp.path())
        .responses(vec![
            scripted_tool_call_response(vec![ToolCallRequest {
                id:        "call_send_1".to_string(),
                name:      "send-file".to_string(),
                arguments: json!({"file_path": file_path_str}).to_string(),
            }]),
            scripted_response("File sent."),
            scripted_response("(padding)"),
            scripted_response("(padding)"),
        ])
        .build()
        .await;

    let principal = Principal::lookup("test".to_string());
    let session_key = tk
        .handle
        .spawn_named(
            &tk.agent_name,
            "send me the test file".to_string(),
            principal,
            None,
        )
        .await
        .expect("spawn session");

    wait_for_turn_count(&tk.handle, session_key, 1).await;

    let traces = tk.handle.get_process_turns(session_key);
    assert_eq!(traces.len(), 1, "should have exactly 1 turn");
    let turn = &traces[0];
    assert!(turn.success, "turn should succeed: {:?}", turn.error);
    // The turn must have at least 2 iterations: tool call + final reply.
    assert!(
        turn.iterations.len() >= 2,
        "expected at least 2 iterations (send-file + reply), got {}",
        turn.iterations.len()
    );

    tk.shutdown();
}
