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

//! Back-compat coverage for the `rara_message_id` → `rara_turn_id` rename
//! (issue #1978). The on-disk tape JSONL format must keep loading entries
//! whose metadata still uses the legacy `rara_message_id` key, while new
//! writes only emit the new `rara_turn_id` key. The `entries_by_turn_id`
//! lookup must accept both keys interchangeably so a single tape file
//! that mixes legacy + new entries returns the full turn.

use rara_kernel::memory::{FileTapeStore, LlmEntryMetadata, TapeService};
use serde_json::json;

/// Scenario: Tape metadata serializes the new turn-id key.
///
/// Given a fresh `LlmEntryMetadata` constructed with `rara_turn_id`
/// "abc-123", serializing to JSON must emit the key `rara_turn_id` and
/// must NOT emit the legacy `rara_message_id` key. Confirms the writer
/// only produces the new key going forward.
#[test]
fn serializes_rara_turn_id() {
    let metadata = LlmEntryMetadata {
        rara_turn_id:      "abc-123".to_owned(),
        usage:             None,
        model:             "test-model".to_owned(),
        iteration:         0,
        stream_ms:         10,
        first_token_ms:    None,
        reasoning_content: None,
    };

    let value = serde_json::to_value(&metadata).expect("serialize should succeed");
    let object = value
        .as_object()
        .expect("metadata serializes to a JSON object");

    assert_eq!(
        object.get("rara_turn_id").and_then(|v| v.as_str()),
        Some("abc-123"),
        "writer must emit the new rara_turn_id key",
    );
    assert!(
        !object.contains_key("rara_message_id"),
        "writer must not emit the legacy rara_message_id key",
    );
}

/// Scenario: Tape metadata deserializes the legacy `rara_message_id` key.
///
/// Given a JSON object using the legacy key set to "legacy-id-xyz",
/// deserialization into `LlmEntryMetadata` must succeed and the parsed
/// `rara_turn_id` must equal that value. Covers the on-disk back-compat
/// contract for tape JSONL files written before issue #1978.
#[test]
fn accepts_legacy_rara_message_id() {
    let raw = json!({
        "rara_message_id": "legacy-id-xyz",
        "model": "legacy-model",
        "iteration": 2,
        "stream_ms": 42,
    });

    let parsed: LlmEntryMetadata =
        serde_json::from_value(raw).expect("legacy key must deserialize");

    assert_eq!(parsed.rara_turn_id, "legacy-id-xyz");
    assert_eq!(parsed.model, "legacy-model");
    assert_eq!(parsed.iteration, 2);
    assert_eq!(parsed.stream_ms, 42);
}

/// Scenario: `entries_by_turn_id` returns all entries of a turn for both
/// legacy and new metadata.
///
/// Given a tape containing two entries whose metadata uses the legacy
/// key `rara_message_id` set to "turn-A" and one entry whose metadata
/// uses the new key `rara_turn_id` also set to "turn-A", invoking
/// `TapeService::entries_by_turn_id` with "turn-A" must return all
/// three entries. This is the single test that nails the rename:
/// before the change the lookup would have missed the new-keyed entry
/// (or, depending on direction, the legacy ones); after the change
/// both keys are honored.
#[tokio::test]
async fn entries_by_turn_id_back_compat() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let store = FileTapeStore::new(tmp.path(), tmp.path())
        .await
        .expect("file store");
    let tape = TapeService::new(store);

    let tape_name = "back-compat-tape";

    // Two entries written with the legacy key shape, as on-disk tapes
    // produced before issue #1978 would have looked.
    tape.append_message(
        tape_name,
        json!({"role": "user", "content": "first legacy entry"}),
        Some(json!({"rara_message_id": "turn-A"})),
    )
    .await
    .expect("append legacy entry 1");

    tape.append_message(
        tape_name,
        json!({"role": "assistant", "content": "second legacy entry"}),
        Some(json!({"rara_message_id": "turn-A"})),
    )
    .await
    .expect("append legacy entry 2");

    // One entry written with the new key shape (post-#1978 producer).
    tape.append_message(
        tape_name,
        json!({"role": "assistant", "content": "third new-key entry"}),
        Some(json!({"rara_turn_id": "turn-A"})),
    )
    .await
    .expect("append new-key entry");

    // Sanity: an unrelated turn must not pollute the result.
    tape.append_message(
        tape_name,
        json!({"role": "user", "content": "unrelated turn"}),
        Some(json!({"rara_turn_id": "turn-B"})),
    )
    .await
    .expect("append unrelated entry");

    let hits = tape
        .entries_by_turn_id(tape_name, "turn-A")
        .await
        .expect("entries_by_turn_id");

    assert_eq!(
        hits.len(),
        3,
        "legacy + new metadata keys must be matched together",
    );
}
