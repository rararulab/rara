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

//! E2E regression — a failed turn must yield a visible turn trace (#2178).
//!
//! Lane 2 (scripted LLM): the driver is scripted to fail with a provider
//! error, so the turn errors out before any LLM iteration completes.
//! Before #2178 the kernel's `handle_turn_completed` Err arm never called
//! `push_turn_trace`, leaving `get_process_turns` empty — a caller (or the
//! real-LLM e2e's wait loop) saw `current_turns=0 latest_trace=None` and
//! had no way to learn the provider error. This test pins the fix: exactly
//! one `success=false` trace carrying the provider error message.
//!
//! Note this is the kernel-boundary complement to #1938, which suppressed
//! the information-FREE all-zero trace in the agent loop's persistence
//! path. The trace asserted here is information-BEARING: it exists solely
//! to carry the error text.

use std::time::Duration;

use rara_kernel::{error::KernelError, identity::Principal, testing::TestKernelBuilder};

/// Marker embedded in the scripted provider error. Deliberately avoids the
/// kernel's retryable / rate-limit / quota patterns (`429`, `rate limit`,
/// `usage limit`, ...) so the agent loop fails the turn on the first driver
/// call instead of entering a recovery iteration.
const SCRIPTED_ERROR: &str = "scripted provider outage e2178";

/// One scripted `Err` → exactly one recorded turn with `success == false`
/// and an `error` that carries the provider message.
///
/// Fails before the #2178 kernel change by timing out: without the Err-arm
/// `push_turn_trace`, `get_process_turns` stays empty forever.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_turn_yields_error_bearing_trace() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let tk = TestKernelBuilder::new(tmp.path())
        .with_results(vec![Err(KernelError::Provider {
            message: SCRIPTED_ERROR.into(),
        })])
        .build()
        .await;

    // `TurnMetrics` (and thus `TurnWaiter`) only fires for successful turns,
    // so poll the process table directly, bounded by a generous deadline.
    let principal = Principal::lookup("test");
    let session_key = tk
        .handle
        .spawn_named("test-agent", "ping".to_string(), principal, None)
        .await
        .expect("spawn agent");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let traces = loop {
        let traces = tk.handle.get_process_turns(session_key);
        if !traces.is_empty() {
            break traces;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for the failed turn's trace — the kernel dropped the turn failure \
             without pushing a turn trace (#2178)"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };

    assert_eq!(traces.len(), 1, "expected exactly one recorded turn");
    let turn = &traces[0];
    assert!(!turn.success, "failed turn must record success=false");
    assert!(
        turn.iterations.is_empty(),
        "failure before any completed iteration must not fabricate iteration traces, got: {:?}",
        turn.iterations
    );
    assert_eq!(turn.total_tool_calls, 0, "no tools ran in the failed turn");

    let error = turn
        .error
        .as_deref()
        .expect("failed trace must carry an error message");
    assert!(
        error.contains(SCRIPTED_ERROR),
        "trace error should surface the provider message verbatim, got: {error}"
    );
    assert!(
        error.starts_with("provider:"),
        "trace error should be prefixed with the outbound error category, got: {error}"
    );

    tk.shutdown();
}
