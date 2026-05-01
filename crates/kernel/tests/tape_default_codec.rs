// Copyright 2025 Rararulab
//
// Licensed under the Apache License, Version 2.0 (the "License").

//! Scenario binding for `tape_store_uses_serde_codec_by_default`
//! (issue #2007). Asserts two things:
//!
//! 1. The `zig-codec` cargo feature is OFF in this build configuration — so no
//!    Zig static archive is linked into the test binary by way of the kernel's
//!    optional `tape-codec-zig` dep.
//! 2. A real tape round-trip (append → re-read) succeeds against a fresh
//!    `FileTapeStore`. This exercises the default codec path through
//!    `serde_json::{to_vec, from_slice}` end-to-end, so any regression in the
//!    feature-gated swap that breaks the default path will be caught here.

#![cfg(not(feature = "zig-codec"))]

use rara_kernel::memory::{FileTapeStore, TapEntryKind};
use serde_json::json;

#[tokio::test]
async fn tape_store_uses_serde_codec_by_default() {
    // The `#![cfg(not(feature = "zig-codec"))]` at the top of this file
    // is the binding for the spec's "feature flag default-off" scenario:
    // this test only compiles and runs when the kernel is built without
    // `--features zig-codec`. If a future change wires the feature into
    // the kernel's default feature set, the test will be silently
    // excluded from default `cargo test` and the spec lifecycle gate
    // will report it as missing — not a perfect tripwire, but adequate
    // for a PoC.

    // Round-trip a small tape through the production code path.
    let tmp = tempfile::tempdir().expect("tempdir");
    let store = FileTapeStore::new(tmp.path(), tmp.path())
        .await
        .expect("FileTapeStore::new");

    let appended = store
        .append(
            "test-tape",
            TapEntryKind::Message,
            json!({"role": "user", "text": "hi"}),
            None,
        )
        .await
        .expect("append");
    assert_eq!(appended.entry.kind, TapEntryKind::Message);

    let read = store
        .read("test-tape")
        .await
        .expect("read")
        .expect("tape exists after append");
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].payload, json!({"role": "user", "text": "hi"}));
}
