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

//! E2E contract — lane 1 (no LLM).
//!
//! Demonstrates the canonical shape of a no-LLM kernel e2e: boot a real
//! [`TestKernelBuilder`] with no scripted LLM responses, route a payload
//! through the kernel along a path that short-circuits before the agent
//! loop is ever consulted, then assert on the persisted [`TapEntry`] and
//! confirm no [`TurnTrace`] was recorded.
//!
//! Companion to `docs/guides/e2e-style.md`. The assertions here read
//! meaningfully without any LLM behavior — that's the lane-1 invariant.
//!
//! Pairs with `e2e_contract_lane2_scripted.rs` (lane 2, scripted LLM).

use rara_kernel::{memory::TapEntryKind, session::SessionKey, testing::TestKernelBuilder};

/// A `Note` written through the kernel's `TapeService` is persisted on
/// the named tape and is readable on a subsequent `entries(..)` call,
/// without the agent loop ever being invoked.
///
/// This is the lane-1 archetype: the assertion is a pure rara-internal
/// state observation (tape contents) that does not depend on, and is not
/// reachable by, any LLM call.
#[tokio::test]
async fn lane1_no_llm_tape_write_persists_without_agent_turn() {
    let tmp = tempfile::tempdir().expect("tempdir");
    // No `.responses(...)` — the `ScriptedLlmDriver` queue is empty.
    // Any path that reaches the agent loop would panic on the empty queue;
    // a clean lane-1 run never gets there.
    let tk = TestKernelBuilder::new(tmp.path()).build().await;

    let session_key = SessionKey::new();
    let tape_name = session_key.to_string();

    // Write directly through the kernel's tape surface — the same
    // `TapeService` the kernel itself uses for every persistent write.
    // This stands in for any short-circuit path (guard rejection,
    // silent-append delivery, manual note insertion) whose net effect is
    // "a tape entry lands without the agent loop firing".
    let written = tk
        .handle
        .tape()
        .store()
        .append(
            &tape_name,
            TapEntryKind::Note,
            serde_json::json!({"content": "lane1 short-circuit"}),
            None,
        )
        .await
        .expect("append note");
    assert_eq!(written.entry.kind, TapEntryKind::Note);

    // Tape contains exactly the one short-circuit entry we wrote.
    let entries = tk
        .handle
        .tape()
        .entries(&tape_name)
        .await
        .expect("read tape");
    assert_eq!(entries.len(), 1, "expected the single Note entry");
    assert_eq!(entries[0].kind, TapEntryKind::Note);
    assert_eq!(
        entries[0].payload,
        serde_json::json!({"content": "lane1 short-circuit"})
    );

    // No agent turn was recorded — the LLM was never consulted.
    let turns = tk.handle.get_process_turns(session_key);
    assert!(
        turns.is_empty(),
        "lane-1 path must not record an agent turn, got {turns:?}"
    );

    tk.shutdown();
}
