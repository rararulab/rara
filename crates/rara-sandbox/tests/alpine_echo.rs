//! End-to-end round-trip: create -> exec `echo` -> destroy.
//!
//! This test is `#[ignore]`d because it requires (a) a real boxlite build
//! (no `BOXLITE_DEPS_STUB`) and (b) a local OCI image store that can
//! resolve `alpine:latest`. CI today builds with the stub for runner
//! provisioning reasons (#1842), so the test cannot run there.
//!
//! Run locally on macOS:
//!
//! ```bash
//! cargo build -p rara-sandbox            # boxlite build.rs downloads runtime
//! cargo run -p rara-cli -- setup boxlite # stage files into user-data dir
//! cargo test -p rara-sandbox -- --ignored alpine_echo_roundtrip
//! ```

use futures::StreamExt;
use rara_sandbox::{ExecRequest, Sandbox, SandboxConfig};

#[tokio::test]
#[ignore = "requires boxlite runtime files (see issue #1699) and a local OCI image cache"]
async fn alpine_echo_roundtrip() {
    let config = SandboxConfig::builder()
        .rootfs_image("alpine:latest".to_owned())
        .build();

    let sandbox = Sandbox::create(config)
        .await
        .expect("sandbox creation should succeed when runtime files are staged");

    let request = ExecRequest::builder()
        .command("echo".to_owned())
        .args(vec!["Hello from BoxLite!".to_owned()])
        .build();

    let mut outcome = sandbox.exec(request).await.expect("exec should succeed");

    let mut lines = Vec::new();
    while let Some(line) = outcome.stdout.next().await {
        lines.push(line);
    }

    assert!(
        lines.iter().any(|l| l.contains("Hello from BoxLite!")),
        "expected echo output, got: {lines:?}"
    );

    sandbox.destroy().await.expect("destroy should succeed");
}
