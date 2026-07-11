// Integration test for the session-scoped lifecycle of the `run_code` tool.
//
// Marked `#[ignore]` for the same reason as
// `crates/rara-sandbox/tests/alpine_echo.rs`: it requires `rara setup boxlite`
// to have staged runtime files plus a warm OCI image cache. CI runs with
// `BOXLITE_DEPS_STUB=1` and would always fail this test.

use std::sync::Arc;

use dashmap::DashMap;
use rara_app::{
    SandboxToolConfig,
    tools::run_code::{RunCodeParams, RunCodeTool, SandboxCleanupHook, SandboxMap},
};
use rara_kernel::{
    io::MessageId,
    lifecycle::{LifecycleHook, SessionEndContext},
    queue::{ShardedEventQueue, ShardedEventQueueConfig},
    session::SessionKey,
    tool::{ToolContext, ToolExecute},
};

#[tokio::test]
#[ignore = "requires boxlite runtime files (issue #1699) and a local OCI image cache"]
async fn run_code_reuses_sandbox_across_calls_and_destroys_on_session_end() {
    let cfg = SandboxToolConfig::builder()
        .default_rootfs_image("alpine:latest".to_owned())
        .build();
    let map: SandboxMap = Arc::new(DashMap::new());
    let tool = RunCodeTool::new(Some(cfg), map.clone());
    let session = SessionKey::default();

    // First call: must create the sandbox.
    let first = tool
        .sandbox_for_session(session)
        .await
        .expect("sandbox creation should succeed");
    assert_eq!(map.len(), 1, "first call must populate the map");

    // Second call: must reuse the same Arc (pointer equality).
    let second = tool
        .sandbox_for_session(session)
        .await
        .expect("second lookup should succeed");
    assert!(
        Arc::ptr_eq(&first, &second),
        "subsequent calls must reuse the existing sandbox"
    );

    // Drop our locally held Arcs so try_unwrap inside the hook succeeds.
    drop(first);
    drop(second);

    // Hook fires destroy in a spawned task; map entry is removed synchronously.
    let hook = SandboxCleanupHook::new(map.clone());
    hook.on_session_end(&SessionEndContext {
        session_key:   session,
        manifest_name: "test".to_owned(),
    })
    .await;
    assert_eq!(map.len(), 0, "session-end hook must remove the entry");
}

/// A non-terminating command is hard-killed by boxlite once the exec timeout
/// elapses, and `run` returns normally with `timed_out = true` — releasing the
/// per-session lock at a bounded point instead of hanging on the kernel wall.
///
/// `#[ignore]`d for the same boxlite-runtime reason as the test above; not
/// bound to a lifecycle scenario (the CI-runnable binding lives in the
/// `run_code` unit tests). This is the real end-to-end backstop.
#[tokio::test]
#[ignore = "requires boxlite runtime files (issue #1699) and a local OCI image cache"]
async fn run_code_times_out_a_runaway_command() {
    let cfg = SandboxToolConfig::builder()
        .default_rootfs_image("alpine:latest".to_owned())
        .build();
    let map: SandboxMap = Arc::new(DashMap::new());
    let tool = RunCodeTool::new(Some(cfg), map.clone());

    let params: RunCodeParams = serde_json::from_value(serde_json::json!({
        "command": "sh",
        "args": ["-c", "while true; do :; done"],
        "timeout": 1,
    }))
    .expect("params parse");

    let ctx = tool_context();

    // boxlite must kill the runaway within a small multiple of the 1s timeout;
    // the outer bound guards against the pre-fix hang.
    let result = tokio::time::timeout(std::time::Duration::from_secs(30), tool.run(params, &ctx))
        .await
        .expect("run must return well before the kernel wall")
        .expect("run must not error");

    assert!(
        result.timed_out,
        "runaway exec must report timed_out = true"
    );
}

/// Minimal [`ToolContext`] for driving a tool's `run` in an integration test.
fn tool_context() -> ToolContext {
    ToolContext {
        user_id:               "test-user".to_owned(),
        session_key:           SessionKey::new(),
        origin_endpoint:       None,
        origin_user_id:        None,
        event_queue:           Arc::new(ShardedEventQueue::new(ShardedEventQueueConfig {
            num_shards:      0,
            shard_capacity:  1,
            global_capacity: 16,
        })),
        rara_turn_id:          MessageId::new(),
        context_window_tokens: 0,
        tool_registry:         None,
        stream_handle:         None,
        tool_call_id:          None,
    }
}
