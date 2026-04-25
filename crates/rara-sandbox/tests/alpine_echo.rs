//! End-to-end round-trip: create -> exec `echo` -> destroy.
//!
//! This test is `#[ignore]`d because it requires the boxlite runtime files
//! (`boxlite-guest`, `libkrunfw.dylib`, `mke2fs`, `boxlite-shim`, `debugfs`)
//! to be staged under the platform-specific runtime directory, and a local
//! OCI image store that can resolve `alpine:latest`. Runtime-file staging
//! is tracked in issue #1699; until that lands, CI cannot run this test
//! from a fresh checkout.
//!
//! Run locally with:
//!
//! ```bash
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
