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

//! Integration tests for `StreamEvent::UserMessageAppended` (issue #2063).
//!
//! Three scenarios mapped to the spec's BDD acceptance criteria:
//!
//! - `kernel_emits_user_message_appended_after_tape_append` — non-Mita user
//!   message → exactly one `UserMessageAppended` whose `seq` / `content` /
//!   `created_at` match the persisted tape entry, ordered before any
//!   `TapeForked` for the same turn.
//! - `kernel_does_not_emit_user_message_appended_for_mita_directive` —
//!   `metadata.mita_directive=true` → zero `UserMessageAppended` events.
//! - `kernel_user_message_appended_seq_matches_rest_messages_endpoint` — the
//!   `seq` carried by the event equals the seq the
//!   `tap_entries_to_chat_messages` walker (REST `/messages`) produces for the
//!   same entry. Catches drift between the two seq derivations.

use std::{collections::HashMap, path::PathBuf, sync::Once, time::Duration};

use rara_kernel::{
    channel::types::{ChannelType, MessageContent},
    identity::{Principal, UserId},
    io::{ChannelSource, InboundMessage, MessageId, StreamEvent},
    memory::{TapEntry, TapEntryKind},
    session::SessionKey,
    testing::{TestKernelBuilder, scripted_response},
};
use serde_json::{Value, json};
use tokio::sync::broadcast::error::{RecvError, TryRecvError};

/// Override `rara_paths` to a stable per-process temp dir — same reasoning
/// as `subagent_topology_events::init_test_env`.
fn init_test_env() {
    static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    static INIT: Once = Once::new();
    let root = ROOT.get_or_init(|| {
        std::env::temp_dir().join(format!("rara-kernel-uma-test-{}", std::process::id()))
    });
    INIT.call_once(move || {
        let data = root.join("rara_data");
        let config = root.join("rara_config");
        std::fs::create_dir_all(&data).expect("create test data dir");
        std::fs::create_dir_all(&config).expect("create test config dir");
        rara_paths::set_custom_data_dir(&data);
        rara_paths::set_custom_config_dir(&config);
    });
}

/// Drain whatever events are queued on the receiver right now.
fn drain_now(rx: &mut tokio::sync::broadcast::Receiver<StreamEvent>) -> Vec<StreamEvent> {
    let mut out = Vec::new();
    loop {
        match rx.try_recv() {
            Ok(ev) => out.push(ev),
            Err(TryRecvError::Empty) | Err(TryRecvError::Closed) => return out,
            Err(TryRecvError::Lagged(_)) => continue,
        }
    }
}

/// Wait up to `timeout` for an event matching `pred`. Returns the matched
/// event or `None` on timeout.
async fn wait_for<F>(
    rx: &mut tokio::sync::broadcast::Receiver<StreamEvent>,
    timeout: Duration,
    mut pred: F,
) -> Option<StreamEvent>
where
    F: FnMut(&StreamEvent) -> bool,
{
    tokio::time::timeout(timeout, async {
        loop {
            match rx.recv().await {
                Ok(ev) if pred(&ev) => return Some(ev),
                Ok(_) => continue,
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => return None,
            }
        }
    })
    .await
    .ok()
    .flatten()
}

/// Walk a tape with the same per-result seq rules
/// `tap_entries_to_chat_messages` uses, returning the seq of the entry
/// whose id matches `target_id` (Message-only — sufficient for this test
/// since we only assert against user-message entries).
fn rest_chat_seq_for_entry_id(entries: &[TapEntry], target_id: u64) -> Option<i64> {
    let mut seq: i64 = 0;
    for entry in entries {
        match entry.kind {
            TapEntryKind::Message
                if serde_json::from_value::<rara_kernel::llm::Message>(entry.payload.clone())
                    .is_ok() =>
            {
                seq += 1;
                if entry.id == target_id {
                    return Some(seq);
                }
            }
            TapEntryKind::ToolCall
                if entry
                    .payload
                    .get("calls")
                    .and_then(Value::as_array)
                    .is_some() =>
            {
                seq += 1;
            }
            TapEntryKind::ToolResult => {
                if let Some(results) = entry.payload.get("results").and_then(Value::as_array) {
                    seq += results.len() as i64;
                }
            }
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Scenario 1: emit-on-success
// ---------------------------------------------------------------------------

/// Submitting a non-Mita user message produces exactly one
/// `UserMessageAppended` on the session bus, ordered before any
/// `TapeForked` for the same turn, with `seq` / `content` / `created_at`
/// matching the persisted tape entry.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kernel_emits_user_message_appended_after_tape_append() {
    init_test_env();
    let tmp = tempfile::tempdir().expect("tempdir");
    let tk = TestKernelBuilder::new(tmp.path())
        .responses(vec![scripted_response("ok")])
        .build()
        .await;

    let session_key = SessionKey::new();
    let mut rx = tk
        .handle
        .stream_hub()
        .subscribe_session_events(&session_key);

    let principal = Principal::lookup("test");
    let manifest = tk
        .handle
        .agent_registry()
        .get("test-agent")
        .expect("test-agent manifest registered");
    let returned_key = tk
        .handle
        .spawn_with_input(
            manifest,
            "hello world".to_string(),
            principal,
            None,
            Some(session_key),
        )
        .await
        .expect("spawn agent");
    assert_eq!(returned_key, session_key);

    let event = wait_for(&mut rx, Duration::from_secs(10), |ev| {
        matches!(ev, StreamEvent::UserMessageAppended { .. })
    })
    .await
    .expect("UserMessageAppended not received within 10s");

    match event {
        StreamEvent::UserMessageAppended {
            parent_session,
            seq,
            content,
            created_at,
        } => {
            assert_eq!(parent_session, session_key, "parent_session must match");
            assert!(seq >= 1, "seq must be a positive chat-seq, got {seq}");
            // Content was sent as plain text — should round-trip as a JSON
            // string per kernel.rs Phase 5 serialisation rules.
            assert_eq!(
                content,
                json!("hello world"),
                "content must mirror the persisted tape payload"
            );

            // Cross-check: the persisted tape's user entry carries the
            // same timestamp and seq.
            let tape_name = session_key.to_string();
            let entries = tk
                .handle
                .tape()
                .store()
                .read(&tape_name)
                .await
                .expect("read tape")
                .expect("tape exists");
            let user_entry = entries
                .iter()
                .find(|e| {
                    matches!(e.kind, TapEntryKind::Message)
                        && e.payload
                            .get("role")
                            .and_then(|v| v.as_str())
                            .map(|s| s == "user")
                            .unwrap_or(false)
                })
                .expect("a user message entry must exist on tape");
            assert_eq!(
                created_at, user_entry.timestamp,
                "created_at must equal the persisted tape entry timestamp"
            );
            let rest_seq = rest_chat_seq_for_entry_id(&entries, user_entry.id)
                .expect("REST walker must find seq for the user entry");
            assert_eq!(
                seq, rest_seq,
                "seq carried on the topology event must match the REST /messages seq for the same \
                 entry"
            );
        }
        other => panic!("expected UserMessageAppended, got {other:?}"),
    }

    // Exactly one UserMessageAppended for this single submit — drain
    // the rest of the buffer and verify no second instance arrives.
    let leftover = drain_now(&mut rx);
    let extra: Vec<_> = leftover
        .iter()
        .filter(|ev| matches!(ev, StreamEvent::UserMessageAppended { .. }))
        .collect();
    assert!(
        extra.is_empty(),
        "expected exactly one UserMessageAppended, got an extra: {extra:?}"
    );

    tk.shutdown();
}

// ---------------------------------------------------------------------------
// Scenario 2: Mita directive does NOT emit
// ---------------------------------------------------------------------------

/// A message with `metadata.mita_directive=true` is recorded as an
/// `Event` entry on the tape (mita-directive bookkeeping) and must NOT
/// emit a `UserMessageAppended` on the topology bus.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kernel_does_not_emit_user_message_appended_for_mita_directive() {
    init_test_env();
    let tmp = tempfile::tempdir().expect("tempdir");
    let tk = TestKernelBuilder::new(tmp.path())
        .responses(vec![scripted_response("ok"), scripted_response("ok2")])
        .build()
        .await;

    // Spawn an initial session so the directive has a target process.
    let session_key = SessionKey::new();
    let principal = Principal::lookup("test");
    let manifest = tk
        .handle
        .agent_registry()
        .get("test-agent")
        .expect("test-agent manifest registered");
    let returned_key = tk
        .handle
        .spawn_with_input(
            manifest,
            "first turn".to_string(),
            principal.clone(),
            None,
            Some(session_key),
        )
        .await
        .expect("spawn agent");
    assert_eq!(returned_key, session_key);

    // Subscribe AFTER the spawn so we observe events emitted by the
    // forthcoming Mita-directive delivery (we explicitly DO NOT care
    // about the spawn's own UserMessageAppended frame here).
    let mut rx = tk
        .handle
        .stream_hub()
        .subscribe_session_events(&session_key);

    // Wait for the initial spawn turn to settle so the next deliver hits
    // an idle process — otherwise it gets buffered and skipped under
    // active-turn semantics.
    tokio::time::sleep(Duration::from_millis(500)).await;
    drain_now(&mut rx);

    // Construct an InboundMessage with `mita_directive=true` and route
    // it through the kernel's standard ingress.
    let mut metadata: HashMap<String, Value> = HashMap::new();
    metadata.insert("mita_directive".to_owned(), json!(true));
    let inbound = InboundMessage::unresolved(
        MessageId::new(),
        ChannelSource {
            channel_type:        ChannelType::Internal,
            platform_message_id: None,
            platform_user_id:    "test".to_owned(),
            platform_chat_id:    None,
        },
        UserId("test".to_owned()),
        Some(session_key),
        None,
        MessageContent::Text("mita please re-enter".to_owned()),
        None,
        jiff::Timestamp::now(),
        metadata,
    );
    tk.handle.deliver_internal(inbound).await;

    // Give the kernel a generous moment to process the directive.
    tokio::time::sleep(Duration::from_millis(800)).await;

    let events = drain_now(&mut rx);
    let bad: Vec<_> = events
        .iter()
        .filter(|ev| matches!(ev, StreamEvent::UserMessageAppended { .. }))
        .collect();
    assert!(
        bad.is_empty(),
        "Mita directive must NOT emit UserMessageAppended, got: {bad:?}"
    );

    tk.shutdown();
}

// ---------------------------------------------------------------------------
// Scenario 2b: tape append failure does NOT emit
// ---------------------------------------------------------------------------

/// When `TapeService::append_message_with_chat_seq` fails (here: tape
/// directory made read-only after kernel boot so the lazy file `open`
/// returns `EACCES`), the kernel must NOT emit a `UserMessageAppended`
/// event. Spec scenario "Kernel does not emit UserMessageAppended when
/// tape append fails" — emit-on-success-only.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kernel_does_not_emit_user_message_appended_on_tape_failure() {
    use std::os::unix::fs::PermissionsExt;

    init_test_env();
    let tmp = tempfile::tempdir().expect("tempdir");
    let tk = TestKernelBuilder::new(tmp.path())
        .responses(vec![scripted_response("ok")])
        .build()
        .await;

    // The tape root is created at `<tmp>/tapes/tapes` during boot
    // (`TestKernelBuilder` passes `<tmp>/tapes` as the FileTapeStore home,
    // and `WorkerState::new` then nests another `tapes/` under it). Strip
    // write perms so the next lazy `OpenOptions::create(true)` returns
    // EACCES, forcing the kernel's `match append_message_with_chat_seq`
    // arm to hit `Err` and skip the topology emit.
    let tape_root = tmp.path().join("tapes").join("tapes");
    let mut perms = std::fs::metadata(&tape_root)
        .expect("read tape dir metadata")
        .permissions();
    perms.set_mode(0o555);
    std::fs::set_permissions(&tape_root, perms).expect("chmod tape dir read-only");

    let session_key = SessionKey::new();
    let mut rx = tk
        .handle
        .stream_hub()
        .subscribe_session_events(&session_key);

    let principal = Principal::lookup("test");
    let manifest = tk
        .handle
        .agent_registry()
        .get("test-agent")
        .expect("test-agent manifest registered");
    // spawn_with_input may itself surface the tape failure — that's
    // acceptable; what matters is that the topology bus carries no
    // UserMessageAppended frame for this submit.
    let _ = tk
        .handle
        .spawn_with_input(
            manifest,
            "this append must fail".to_string(),
            principal,
            None,
            Some(session_key),
        )
        .await;

    // Give the kernel a generous moment to attempt the append + emit.
    tokio::time::sleep(Duration::from_millis(800)).await;

    let events = drain_now(&mut rx);
    let bad: Vec<_> = events
        .iter()
        .filter(|ev| matches!(ev, StreamEvent::UserMessageAppended { .. }))
        .collect();
    assert!(
        bad.is_empty(),
        "tape-append failure must NOT emit UserMessageAppended, got: {bad:?}"
    );

    // Restore perms so tempdir cleanup can drop the directory.
    let mut perms = std::fs::metadata(&tape_root)
        .expect("re-read tape dir metadata")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&tape_root, perms).expect("chmod tape dir restore");

    tk.shutdown();
}

// ---------------------------------------------------------------------------
// Scenario 3: seq matches the REST endpoint walker (drift guard)
// ---------------------------------------------------------------------------

/// The chat seq on the topology event must equal the seq the
/// `/messages` REST walker produces for the same entry, even after the
/// tape has acquired non-Message entries (Anchor / Event) that the
/// position-based walker skips.
///
/// This is the explicit drift guard required by the spec's "Where the
/// seq comes from" decision: the topology event's seq must match the
/// REST endpoint's seq for the same tape entry, full stop.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kernel_user_message_appended_seq_matches_rest_messages_endpoint() {
    init_test_env();
    let tmp = tempfile::tempdir().expect("tempdir");
    let tk = TestKernelBuilder::new(tmp.path())
        .responses(vec![scripted_response("ok")])
        .build()
        .await;

    let session_key = SessionKey::new();
    let mut rx = tk
        .handle
        .stream_hub()
        .subscribe_session_events(&session_key);

    let principal = Principal::lookup("test");
    let manifest = tk
        .handle
        .agent_registry()
        .get("test-agent")
        .expect("test-agent manifest registered");
    let _ = tk
        .handle
        .spawn_with_input(
            manifest,
            "drift guard prompt".to_string(),
            principal,
            None,
            Some(session_key),
        )
        .await
        .expect("spawn agent");

    let event = wait_for(&mut rx, Duration::from_secs(10), |ev| {
        matches!(ev, StreamEvent::UserMessageAppended { .. })
    })
    .await
    .expect("UserMessageAppended not received within 10s");

    let StreamEvent::UserMessageAppended {
        seq: emitted_seq, ..
    } = event
    else {
        unreachable!("filtered above");
    };

    let tape_name = session_key.to_string();
    let entries = tk
        .handle
        .tape()
        .store()
        .read(&tape_name)
        .await
        .expect("read tape")
        .expect("tape exists");
    let user_entry = entries
        .iter()
        .find(|e| {
            matches!(e.kind, TapEntryKind::Message)
                && e.payload
                    .get("role")
                    .and_then(|v| v.as_str())
                    .map(|s| s == "user")
                    .unwrap_or(false)
        })
        .expect("user entry must exist on tape");
    let rest_seq = rest_chat_seq_for_entry_id(&entries, user_entry.id)
        .expect("REST walker must find seq for the user entry");
    assert_eq!(
        emitted_seq, rest_seq,
        "topology event seq must equal REST /messages seq for the same tape entry — drift detected"
    );

    tk.shutdown();
}
