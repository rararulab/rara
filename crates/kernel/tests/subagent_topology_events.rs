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

//! Multi-agent topology `StreamEvent` integration tests for issue #1999.
//!
//! Each test boots a real kernel via [`TestKernelBuilder`], drives a
//! topology transition through the same code path production uses, and
//! observes the emitted events on a session-level bus subscriber. The
//! four tested transitions are:
//!
//! - child spawn → `SubagentSpawned` on the parent's bus
//! - root spawn (no parent) → no `SubagentSpawned` is emitted
//! - child completion → `SubagentDone` on the parent's bus
//! - agent-turn tape fork → `TapeForked` on the session's bus

use std::{path::PathBuf, sync::Once, time::Duration};

use rara_kernel::{
    identity::Principal,
    io::StreamEvent,
    session::SessionKey,
    testing::{TestKernelBuilder, scripted_response},
};
use tokio::sync::broadcast::error::{RecvError, TryRecvError};

/// Override `rara_paths` to a stable per-process temp dir — same reasoning
/// as `e2e_contract_lane2_scripted::init_test_env`. Needed because the
/// agent loop touches `rara_paths::workspace_dir()` during system-prompt
/// build, and the Linux ARC runner has a read-only `$HOME`.
fn init_test_env() {
    static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    static INIT: Once = Once::new();
    let root = ROOT.get_or_init(|| {
        std::env::temp_dir().join(format!("rara-kernel-topo-test-{}", std::process::id()))
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

/// Drain whatever events are queued on the receiver right now, returning
/// them as a `Vec`. Non-blocking — used by tests that have already done
/// the synchronous work that should have produced the event.
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

/// Wait up to `timeout` for an event satisfying `pred` to arrive on the
/// receiver. Returns the matched event or `None` on timeout.
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
                // Lagged → some events were dropped because the
                // broadcast buffer overflowed. Keep polling — the event
                // we want may be in the still-live tail.
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => return None,
            }
        }
    })
    .await
    .ok()
    .flatten()
}

// ---------------------------------------------------------------------------
// Scenario 1: SubagentSpawned on parent bus
// ---------------------------------------------------------------------------

/// `parent_id = Some(parent)` on a spawn produces a `SubagentSpawned`
/// topology event on the parent's session bus whose three fields match
/// the spawn's identifiers.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subagent_spawned_event_on_parent_bus() {
    init_test_env();
    let tmp = tempfile::tempdir().expect("tempdir");
    // Two scripted responses: one for the parent's first turn, one for the
    // child's first turn. Both will be consumed regardless of test outcome
    // because the agent loops kick off async.
    let tk = TestKernelBuilder::new(tmp.path())
        .responses(vec![
            scripted_response("parent ok"),
            scripted_response("child ok"),
        ])
        .build()
        .await;

    let principal = Principal::lookup("test");
    let parent_key = tk
        .handle
        .spawn_named("test-agent", "ping".to_string(), principal.clone(), None)
        .await
        .expect("spawn parent");

    // Subscribe AFTER parent exists so the bus is bound to a real session.
    let mut rx = tk.handle.stream_hub().subscribe_session_events(&parent_key);

    // Spawn the child via the same kernel surface `spawn_child` uses
    // internally — but invoke `spawn_with_input` directly so we get the
    // child's session key back without needing the AgentHandle's mpsc
    // result_rx in this test.
    let manifest = tk
        .handle
        .agent_registry()
        .get("test-agent")
        .expect("test-agent manifest registered by TestKernelBuilder");
    let child_key = tk
        .handle
        .spawn_with_input(
            manifest,
            "child ping".to_string(),
            principal,
            Some(parent_key),
            None,
        )
        .await
        .expect("spawn child");

    let event = wait_for(&mut rx, Duration::from_secs(5), |ev| {
        matches!(ev, StreamEvent::SubagentSpawned { child_session, .. } if *child_session == child_key)
    })
    .await
    .expect("SubagentSpawned not received within 5s");

    match event {
        StreamEvent::SubagentSpawned {
            parent_session,
            child_session,
            manifest_name,
        } => {
            assert_eq!(
                parent_session, parent_key,
                "parent_session must match the spawn's parent_id"
            );
            assert_eq!(
                child_session, child_key,
                "child_session must match the spawn's returned key"
            );
            assert_eq!(
                manifest_name, "test-agent",
                "manifest_name must match the spawned manifest"
            );
        }
        other => panic!("expected SubagentSpawned, got {other:?}"),
    }

    tk.shutdown();
}

// ---------------------------------------------------------------------------
// Scenario 2: root spawn does NOT emit SubagentSpawned
// ---------------------------------------------------------------------------

/// A root spawn (no parent) must not emit `SubagentSpawned` — the
/// `parent_id.is_none()` branch in `handle_spawn_agent` must NOT
/// publish a topology event on the spawned session's own bus.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subagent_spawned_not_emitted_for_root_spawn() {
    init_test_env();
    let tmp = tempfile::tempdir().expect("tempdir");
    let tk = TestKernelBuilder::new(tmp.path())
        .responses(vec![scripted_response("root ok")])
        .build()
        .await;

    // Subscribe BEFORE the spawn on a known session key, so any
    // SubagentSpawned (incorrectly) emitted on the spawned session's own
    // bus would land here.
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
            "root ping".to_string(),
            principal,
            None, // root spawn — no parent
            Some(session_key),
        )
        .await
        .expect("spawn root");
    assert_eq!(
        returned_key, session_key,
        "desired_session_key must be honoured"
    );

    // Give the kernel a moment to publish anything it would publish.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let events = drain_now(&mut rx);
    let bad: Vec<_> = events
        .iter()
        .filter(|ev| matches!(ev, StreamEvent::SubagentSpawned { .. }))
        .collect();
    assert!(
        bad.is_empty(),
        "root spawn must not emit SubagentSpawned, got: {bad:?}"
    );

    tk.shutdown();
}

// ---------------------------------------------------------------------------
// Scenario 3: SubagentDone on parent bus
// ---------------------------------------------------------------------------

/// Child completion emits `SubagentDone` on the parent's session bus —
/// when `KernelEvent::ChildSessionDone` is processed by
/// `handle_child_completed`, we surface a topology completion event whose
/// `success` mirrors the child's `AgentRunLoopResult.success`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subagent_done_event_on_parent_bus() {
    init_test_env();
    let tmp = tempfile::tempdir().expect("tempdir");
    let tk = TestKernelBuilder::new(tmp.path())
        .responses(vec![
            scripted_response("parent ok"),
            scripted_response("child ok"),
        ])
        .build()
        .await;

    let principal = Principal::lookup("test");
    let parent_key = tk
        .handle
        .spawn_named("test-agent", "ping".to_string(), principal.clone(), None)
        .await
        .expect("spawn parent");

    let mut rx = tk.handle.stream_hub().subscribe_session_events(&parent_key);

    // Resolve the principal once so spawn_child can take a `&Principal<Resolved>`.
    let resolved = tk
        .handle
        .security()
        .resolve_principal(&principal)
        .await
        .expect("resolve principal");
    let manifest = tk
        .handle
        .agent_registry()
        .get("test-agent")
        .expect("test-agent manifest registered");
    let child = tk
        .handle
        .spawn_child(parent_key, &resolved, manifest, "child ping".to_string())
        .await
        .expect("spawn child");

    let child_key = child.session_key;

    let event = wait_for(&mut rx, Duration::from_secs(15), |ev| {
        matches!(ev, StreamEvent::SubagentDone { child_session, .. } if *child_session == child_key)
    })
    .await
    .expect("SubagentDone not received within 15s");

    match event {
        StreamEvent::SubagentDone {
            parent_session,
            child_session,
            success: _,
        } => {
            assert_eq!(parent_session, parent_key, "parent_session must match");
            assert_eq!(child_session, child_key, "child_session must match");
            // `success` is whatever the run loop produced; the spec binds
            // the field's *origin* (AgentRunLoopResult.success), not a
            // particular value — assertion would couple the test to
            // unrelated agent-loop behaviour.
        }
        other => panic!("expected SubagentDone, got {other:?}"),
    }

    tk.shutdown();
}

// ---------------------------------------------------------------------------
// Scenario 4: TapeForked at the agent-turn fork site
// ---------------------------------------------------------------------------

/// Forking a tape inside an agent turn emits `TapeForked` on the
/// session bus — the agent-turn transactional fork at the call site
/// (kernel.rs `tape_service.store().fork(&tape_name, None)`) publishes
/// a `TapeForked` whose `forked_from` / `child_tape` align with the
/// store's allocation and `forked_at_anchor` is `None` for the unanchored
/// fork.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tape_forked_event_emitted_on_fork_call_site() {
    init_test_env();
    let tmp = tempfile::tempdir().expect("tempdir");
    let tk = TestKernelBuilder::new(tmp.path())
        .responses(vec![scripted_response("turn ok")])
        .build()
        .await;

    // Use a known session key so we can subscribe before the spawn.
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
            "ping".to_string(),
            principal,
            None,
            Some(session_key),
        )
        .await
        .expect("spawn agent");
    assert_eq!(returned_key, session_key);

    let event = wait_for(&mut rx, Duration::from_secs(10), |ev| {
        matches!(ev, StreamEvent::TapeForked { .. })
    })
    .await
    .expect("TapeForked not received within 10s");

    match event {
        StreamEvent::TapeForked {
            parent_session,
            forked_from,
            child_tape,
            forked_at_anchor,
        } => {
            assert_eq!(
                parent_session, session_key,
                "parent_session must match the session that owns the fork"
            );
            assert_eq!(
                forked_from,
                session_key.to_string(),
                "forked_from must equal the session's tape name"
            );
            assert!(
                !child_tape.is_empty(),
                "child_tape must be the allocated fork name (non-empty)"
            );
            assert_ne!(
                child_tape, forked_from,
                "child_tape must differ from forked_from"
            );
            assert!(
                forked_at_anchor.is_none(),
                "agent-turn fork is unanchored, expected None, got {forked_at_anchor:?}"
            );
        }
        other => panic!("expected TapeForked, got {other:?}"),
    }

    tk.shutdown();
}
