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

//! E2E contract — lane 2 (scripted LLM via dependency injection).
//!
//! Demonstrates the canonical shape of a kernel-DI scripted-LLM e2e:
//! boot a [`TestKernelBuilder`] with a single deterministic
//! [`ScriptedLlmDriver`] response, drive one agent turn, then assert on
//! the resulting [`TurnTrace`] shape.
//!
//! The scripted driver is **dependency injection at the trait
//! boundary**, not an HTTP fake: the kernel's `LlmSubsys` is a pure Rust
//! trait, so the test simply hands it a different `Arc<dyn LlmDriver>`.
//! Wiremock / mockito / any HTTP-level fake is forbidden — see
//! `docs/guides/e2e-style.md`.
//!
//! Companion to `e2e_contract_lane1_no_llm.rs` (lane 1, no LLM).

use std::{path::PathBuf, sync::Once, time::Duration};

use rara_kernel::{
    identity::Principal,
    testing::{TestKernelBuilder, scripted_response},
};

/// Override `rara_paths` to a stable per-process temp dir so the kernel
/// doesn't try to create `~/.config/rara/workspace` — on the Linux ARC
/// runner `HOME` is read-only and `workspace_dir()` would otherwise
/// fail. Mirrors `web_session_smoke::init_test_env`.
fn init_test_env() {
    static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    static INIT: Once = Once::new();
    let root = ROOT.get_or_init(|| {
        let dir =
            std::env::temp_dir().join(format!("rara-kernel-lane2-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create test env root");
        dir
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

/// One scripted response → exactly one recorded turn whose preview
/// reflects the scripted text. The `TurnTrace.success` flag is true and
/// `iterations.len() == 1` because the scripted response carries a
/// `Stop` reason and no tool calls — so the agent loop exits after a
/// single iteration.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lane2_scripted_single_turn_records_expected_trace() {
    init_test_env();
    let tmp = tempfile::tempdir().expect("tempdir");
    let tk = TestKernelBuilder::new(tmp.path())
        .responses(vec![scripted_response("scripted hello")])
        .build()
        .await;

    // `spawn_named` posts a `SpawnAgent` event to the kernel's queue and
    // awaits the resulting session key — the tightest single-turn entry
    // point that reaches the LLM driver.
    let principal = Principal::lookup("test");
    let session_key = tk
        .handle
        .spawn_named("test-agent", "ping".to_string(), principal, None)
        .await
        .expect("spawn agent");

    // Wait for the agent loop to record a turn via the per-session event
    // bus rather than wall-clock polling — the kernel emits `TurnMetrics`
    // immediately before pushing the turn trace, so on return the trace
    // table is guaranteed populated.
    tk.watch_turn(session_key)
        .wait(Duration::from_secs(30))
        .await
        .expect("turn metrics");
    let traces = tk.handle.get_process_turns(session_key);

    assert_eq!(traces.len(), 1, "expected exactly one recorded turn");
    let turn = &traces[0];
    assert!(turn.success, "turn should succeed: {:?}", turn.error);
    assert_eq!(
        turn.iterations.len(),
        1,
        "Stop-reason scripted response → single iteration"
    );
    let preview = &turn.iterations[0].text_preview;
    assert!(
        preview.contains("scripted hello"),
        "expected preview to contain scripted text, got: {preview}"
    );
    assert_eq!(
        turn.total_tool_calls, 0,
        "scripted response had no tool calls"
    );

    tk.shutdown();
}
