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
    tools::run_code::{RunCodeTool, SandboxCleanupHook, SandboxMap},
};
use rara_kernel::{
    lifecycle::{LifecycleHook, SessionEndContext},
    session::SessionKey,
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
