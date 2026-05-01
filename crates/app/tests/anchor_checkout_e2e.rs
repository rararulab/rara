use std::time::Duration;

use rara_app::{AppConfig, StartOptions, start_with_options};
use rara_kernel::{
    channel::types::{ChannelType, MessageContent},
    identity::{Principal, UserId},
    io::{ChannelSource, InboundMessage, MessageId},
    memory::{FileTapeStore, TapEntryKind, TapeService},
    session::SessionKey,
};
use tokio::time::{Instant, sleep};

const TURN_TIMEOUT_SECS: u64 = 60;

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

#[tokio::test]
#[ignore = "uses the real LLM provider — run with: cargo test -p rara-app --test \
            anchor_checkout_e2e -- --ignored --nocapture"]
async fn anchor_checkout_roundtrip() {
    // 1. Setup — load workspace config + start the kernel under test.
    common_telemetry::logging::init_default_ut_logging();
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::env::set_current_dir(&workspace_root).expect("should switch to workspace root");

    let mut config = AppConfig::new().expect("config should load");
    config.http.bind_address = "127.0.0.1:0".to_string();
    config.grpc.bind_address = "127.0.0.1:0".to_string();
    config.grpc.server_address = "127.0.0.1:0".to_string();
    config.gateway = None;
    config.telegram = None;

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
        .spawn_named(
            "rara",
            "记住这个数字：42是宇宙的答案。确认你记住了。".to_string(),
            principal.clone(),
            None,
        )
        .await
        .expect("should spawn session");

    let chat_id = format!("e2e-checkout-{}", uuid::Uuid::new_v4());
    wait_for_turn_count(&handle, session_key, 1).await;

    // Verify turn 1 succeeded
    let traces = handle.get_process_turns(session_key);
    assert!(traces.last().unwrap().success, "turn 1 should succeed");

    // 3. Send another message to build context
    handle
        .submit_message(build_test_message(
            Some(session_key),
            &chat_id,
            "现在我们讨论一个新话题：Rust的所有权系统。简短解释下 borrow checker 的核心规则。",
        ))
        .expect("msg 2 should submit");
    wait_for_turn_count(&handle, session_key, 2).await;

    // 4. Verify tape has entries and find anchors
    let tape_service = TapeService::new(
        FileTapeStore::new(rara_paths::memory_dir(), &workspace_root)
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
        .submit_message(build_test_message(
            Some(session_key),
            &chat_id,
            "42是什么的答案？只回答数字和含义。",
        ))
        .expect("recall msg should submit");
    wait_for_turn_count(&handle, session_key, 3).await;

    let traces = handle.get_process_turns(session_key);
    let recall_trace = traces.last().unwrap();
    assert!(recall_trace.success, "recall turn should succeed");
    let preview = recall_trace
        .iterations
        .last()
        .map(|i| i.text_preview.clone())
        .unwrap_or_default();
    eprintln!("recall response: {preview}");
    // The LLM should remember "42" from the original context
    assert!(
        preview.contains("42"),
        "LLM should recall the fact from original session, got: {preview}"
    );

    eprintln!("E2E anchor checkout test passed!");
    app.shutdown();
}
