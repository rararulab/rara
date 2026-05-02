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

//! Integration tests for the anchor-segment chat history endpoint
//! (issue #2040).
//!
//! Each test bound to a Gherkin scenario in
//! `specs/issue-2040-anchor-segment-chat-history.spec.md` shares a
//! prefix matching the spec's `Test:` `Filter:` line so
//! `agent-spec lifecycle` can resolve the binding via cargo's
//! positional substring filter.

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use axum::{http::StatusCode, response::IntoResponse};
use chrono::Utc;
use rara_backend_admin::chat::{error::ChatError, service::SessionService};
use rara_domain_shared::settings::SettingsProvider;
use rara_kernel::{
    channel::types::ChannelType,
    llm::{LlmModelLister, ModelInfo},
    memory::{FileTapeStore, HandoffState, TapeService},
    session::{
        ChannelBinding, SessionDerivedState, SessionError, SessionIndex, SessionKey,
        SessionListFilter, SessionStatus, test_utils::InMemorySessionIndex,
    },
    trace::TraceService,
};
use rara_sessions::types::SessionEntry;
use serde_json::json;

// -- session-index wrapper that honours `update_session_derived` --------------
//
// The kernel-shipped `InMemorySessionIndex` (`crates/kernel/src/session/**` is
// Forbidden by this spec's Boundaries, so we cannot extend it in place) leaves
// `update_session_derived` as the trait's default no-op. That breaks the
// anchor-segment tests because the segment-fetch path resolves anchor offsets
// from the session row's `anchors[]`, which is populated only via that method.
// This local wrapper forwards the derived-state update into the inner row so
// the in-memory index behaves like the SQLite-backed one for the fields this
// spec actually reads.
struct DerivedSessionIndex {
    inner: Arc<InMemorySessionIndex>,
}

#[async_trait]
impl SessionIndex for DerivedSessionIndex {
    async fn create_session(&self, entry: &SessionEntry) -> Result<SessionEntry, SessionError> {
        self.inner.create_session(entry).await
    }

    async fn get_session(&self, key: &SessionKey) -> Result<Option<SessionEntry>, SessionError> {
        self.inner.get_session(key).await
    }

    async fn list_sessions(
        &self,
        limit: i64,
        offset: i64,
        filter: SessionListFilter,
    ) -> Result<Vec<SessionEntry>, SessionError> {
        self.inner.list_sessions(limit, offset, filter).await
    }

    async fn update_session(&self, entry: &SessionEntry) -> Result<SessionEntry, SessionError> {
        self.inner.update_session(entry).await
    }

    async fn update_session_derived(
        &self,
        key: &SessionKey,
        derived: &SessionDerivedState,
    ) -> Result<(), SessionError> {
        // Mirror the SQLite implementation: load → patch the derived
        // fields → store. Missing rows are a silent no-op per the trait
        // contract.
        let Some(mut row) = self.inner.get_session(key).await? else {
            return Ok(());
        };
        row.total_entries = derived.total_entries;
        row.last_token_usage = derived.last_token_usage;
        row.estimated_context_tokens = derived.estimated_context_tokens;
        row.entries_since_last_anchor = derived.entries_since_last_anchor;
        row.anchors = derived.anchors.clone();
        if let Some(p) = derived.preview.clone() {
            row.preview = Some(p);
        }
        // The append timestamp lives on the derived state — propagate it
        // so consumers that sort by `updated_at` see the latest write.
        row.updated_at = derived.updated_at;
        let _ = self.inner.update_session(&row).await?;
        Ok(())
    }

    async fn delete_session(&self, key: &SessionKey) -> Result<(), SessionError> {
        self.inner.delete_session(key).await
    }

    async fn bind_channel(&self, binding: &ChannelBinding) -> Result<ChannelBinding, SessionError> {
        self.inner.bind_channel(binding).await
    }

    async fn get_channel_binding(
        &self,
        channel_type: ChannelType,
        chat_id: &str,
        thread_id: Option<&str>,
    ) -> Result<Option<ChannelBinding>, SessionError> {
        self.inner
            .get_channel_binding(channel_type, chat_id, thread_id)
            .await
    }

    async fn unbind_session(&self, key: &SessionKey) -> Result<(), SessionError> {
        self.inner.unbind_session(key).await
    }
}

// -- shared stubs -------------------------------------------------------------

struct StubSettings;

#[async_trait]
impl SettingsProvider for StubSettings {
    async fn get(&self, _key: &str) -> Option<String> { None }

    async fn set(&self, _key: &str, _value: &str) -> anyhow::Result<()> { Ok(()) }

    async fn delete(&self, _key: &str) -> anyhow::Result<()> { Ok(()) }

    async fn list(&self) -> HashMap<String, String> { HashMap::new() }

    async fn batch_update(&self, _patches: HashMap<String, Option<String>>) -> anyhow::Result<()> {
        Ok(())
    }

    fn subscribe(&self) -> tokio::sync::watch::Receiver<()> {
        let (_tx, rx) = tokio::sync::watch::channel(());
        rx
    }
}

struct StubModelLister;

#[async_trait]
impl LlmModelLister for StubModelLister {
    async fn list_models(&self) -> rara_kernel::error::Result<Vec<ModelInfo>> { Ok(Vec::new()) }
}

// -- fixture ------------------------------------------------------------------

/// Bundle of dependencies the tests share. The `TempDir` is held so the
/// tape root stays alive for the test's duration — `FileTapeStore`
/// keeps the worker thread running and reads through the path.
struct Fixture {
    service:      SessionService,
    tape_service: TapeService,
    sessions:     Arc<InMemorySessionIndex>,
    _tmp:         tempfile::TempDir,
}

async fn build_fixture() -> Fixture {
    let tmp = tempfile::tempdir().expect("tempdir");
    let pool = rara_kernel::testing::build_memory_diesel_pools().await;
    let store = FileTapeStore::new(tmp.path(), tmp.path())
        .await
        .expect("tape store");
    let inner_sessions: Arc<InMemorySessionIndex> = Arc::new(InMemorySessionIndex::new());
    let derived_index: Arc<DerivedSessionIndex> = Arc::new(DerivedSessionIndex {
        inner: inner_sessions.clone(),
    });
    // `with_session_index` wires the derived-state update path so every
    // append persists `anchors[]` (the byte offsets the segment-fetch
    // path then resolves against).
    let tape_service =
        TapeService::with_fts(store, pool.clone()).with_session_index(derived_index.clone());
    let trace_service = TraceService::new(pool);
    let service = SessionService::new(
        derived_index,
        tape_service.clone(),
        trace_service,
        Arc::new(StubSettings),
        Arc::new(StubModelLister),
    );
    Fixture {
        service,
        tape_service,
        sessions: inner_sessions,
        _tmp: tmp,
    }
}

/// Register a session row so the service's `get_session` resolves it.
async fn register_session(sessions: &InMemorySessionIndex, key: &SessionKey) {
    let now = Utc::now();
    let entry = SessionEntry {
        key: *key,
        title: None,
        model: None,
        model_provider: None,
        thinking_level: None,
        system_prompt: None,
        total_entries: 0,
        preview: None,
        last_token_usage: None,
        estimated_context_tokens: 0,
        entries_since_last_anchor: 0,
        anchors: Vec::new(),
        metadata: None,
        status: SessionStatus::Active,
        created_at: now,
        updated_at: now,
    };
    sessions.create_session(&entry).await.expect("register");
}

/// Append `count` user messages to the tape and return their tape entry
/// IDs in order.
async fn append_messages(tape: &TapeService, name: &str, count: usize) -> Vec<u64> {
    let mut ids = Vec::with_capacity(count);
    for i in 0..count {
        let entry = tape
            .append_message(
                name,
                json!({"role": "user", "content": format!("entry-{i}")}),
                None,
            )
            .await
            .expect("append");
        ids.push(entry.id);
    }
    ids
}

/// Append a `session/start`-style anchor and return its
/// `(anchor_id, byte_offset)` from the freshly-resolved session row.
async fn append_anchor(
    tape: &TapeService,
    sessions: &InMemorySessionIndex,
    key: &SessionKey,
    name: &str,
) -> (u64, u64) {
    let _ = tape
        .handoff(
            &key.to_string(),
            name,
            HandoffState {
                owner: Some("test".into()),
                ..Default::default()
            },
        )
        .await
        .expect("anchor append");
    let session = sessions
        .get_session(key)
        .await
        .expect("get_session")
        .expect("session row exists");
    let anchor = session
        .anchors
        .iter()
        .find(|a| a.name == name)
        .cloned()
        .expect("anchor recorded in session row");
    (anchor.anchor_id, anchor.byte_offset)
}

// -- scenario tests -----------------------------------------------------------

/// Scenario 1: half-open `[A2, A3)` — the line at A3 must NOT appear,
/// the line at A2 MUST appear.
#[tokio::test]
async fn segment_between_two_anchors() {
    let fx = build_fixture().await;
    let key = SessionKey::new();
    register_session(&fx.sessions, &key).await;
    let tape = key.to_string();

    // Tape layout: msg msg ANCHOR_A1 msg msg ANCHOR_A2 msg msg ANCHOR_A3 msg
    append_messages(&fx.tape_service, &tape, 2).await;
    let (_a1, _a1_off) = append_anchor(&fx.tape_service, &fx.sessions, &key, "A1").await;
    append_messages(&fx.tape_service, &tape, 2).await;
    let (a2, _a2_off) = append_anchor(&fx.tape_service, &fx.sessions, &key, "A2").await;
    append_messages(&fx.tape_service, &tape, 2).await;
    let (a3, _a3_off) = append_anchor(&fx.tape_service, &fx.sessions, &key, "A3").await;
    append_messages(&fx.tape_service, &tape, 1).await;

    let segment = fx
        .service
        .list_messages_between_anchors(&key, Some(a2), Some(a3))
        .await
        .expect("segment read");

    // The segment captures only ChatMessage-shaped entries; anchors are
    // not in `ChatMessage` shape (they're system metadata). Within the
    // half-open `[A2, A3)` window we expect exactly the two `Message`
    // entries that were appended between A2 and A3.
    assert_eq!(
        segment.len(),
        2,
        "must include exactly the two messages between A2 and A3, got {} entries: {:?}",
        segment.len(),
        segment
            .iter()
            .map(|m| match &m.content {
                rara_kernel::channel::types::MessageContent::Text(s) => s.clone(),
                _ => String::new(),
            })
            .collect::<Vec<_>>()
    );
}

/// Scenario 2: only `from_anchor` → read to EOF.
#[tokio::test]
async fn segment_from_anchor_to_eof() {
    let fx = build_fixture().await;
    let key = SessionKey::new();
    register_session(&fx.sessions, &key).await;
    let tape = key.to_string();

    let _ = append_anchor(&fx.tape_service, &fx.sessions, &key, "A1").await;
    append_messages(&fx.tape_service, &tape, 1).await;
    let (a2, _) = append_anchor(&fx.tape_service, &fx.sessions, &key, "A2").await;
    append_messages(&fx.tape_service, &tape, 3).await;

    let segment = fx
        .service
        .list_messages_between_anchors(&key, Some(a2), None)
        .await
        .expect("segment read");

    // Three messages were appended after A2; the upper bound is EOF.
    assert_eq!(segment.len(), 3, "must read every message after A2 to EOF");
}

/// Scenario 3: only `to_anchor` → read from offset 0.
#[tokio::test]
async fn segment_from_start_to_anchor() {
    let fx = build_fixture().await;
    let key = SessionKey::new();
    register_session(&fx.sessions, &key).await;
    let tape = key.to_string();

    append_messages(&fx.tape_service, &tape, 4).await;
    let (a1, _) = append_anchor(&fx.tape_service, &fx.sessions, &key, "A1").await;
    append_messages(&fx.tape_service, &tape, 2).await;

    let segment = fx
        .service
        .list_messages_between_anchors(&key, None, Some(a1))
        .await
        .expect("segment read");

    // Four messages were appended before A1; nothing past A1 should
    // appear because the upper bound is exclusive.
    assert_eq!(
        segment.len(),
        4,
        "must read messages from offset 0 up to A1"
    );
}

/// Scenario 4: no anchor params → identical to legacy `?limit=N` path.
///
/// We assert the response shape against the legacy `list_messages`
/// directly. The router's no-anchor branch routes to that exact same
/// method (see `list_messages` in `chat/router.rs`), so equality here
/// is the regression-pin.
#[tokio::test]
async fn no_anchor_params_preserves_legacy_behavior() {
    let fx = build_fixture().await;
    let key = SessionKey::new();
    register_session(&fx.sessions, &key).await;
    let tape = key.to_string();

    let _ = append_anchor(&fx.tape_service, &fx.sessions, &key, "A1").await;
    append_messages(&fx.tape_service, &tape, 6).await;

    let legacy = fx
        .service
        .list_messages(&key, 50)
        .await
        .expect("legacy read");

    // The router's `(None, None) =>` arm calls `list_messages` directly.
    // Asserting the legacy method itself returns the expected shape is
    // the surface the regression pin protects.
    assert_eq!(legacy.len(), 6, "legacy `?limit` returns the user messages");

    // And the segment helper must NOT be called when both params are
    // absent — but to make that explicit at the contract level, we
    // additionally confirm that calling the segment helper with both
    // bounds None yields the same set of messages (just to prove the
    // tape was readable end-to-end). Routing-level dispatch is exercised
    // implicitly by the router test fabric.
    let full_segment = fx
        .service
        .list_messages_between_anchors(&key, None, None)
        .await
        .expect("full-range segment");
    assert_eq!(
        full_segment.len(),
        legacy.len(),
        "segment helper with no bounds covers the same messages as the legacy path"
    );
}

/// Scenario 5: unknown anchor id → 404, message names id + session key.
#[tokio::test]
async fn unknown_anchor_returns_404() {
    let fx = build_fixture().await;
    let key = SessionKey::new();
    register_session(&fx.sessions, &key).await;

    let err = fx
        .service
        .list_messages_between_anchors(&key, Some(99999), None)
        .await
        .expect_err("must error on unknown anchor");

    let msg = err.to_string();
    assert!(
        msg.contains("99999") && msg.contains(&key.to_string()),
        "error must name both the anchor id and the session key, got: {msg}"
    );
    let response = err.into_response();
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "unknown anchor must surface as 404"
    );
}

/// Scenario 6: `from_anchor` ordered after `to_anchor` → 400.
#[tokio::test]
async fn reversed_anchors_returns_400() {
    let fx = build_fixture().await;
    let key = SessionKey::new();
    register_session(&fx.sessions, &key).await;
    let tape = key.to_string();

    let (a1, a1_off) = append_anchor(&fx.tape_service, &fx.sessions, &key, "A1").await;
    append_messages(&fx.tape_service, &tape, 2).await;
    let (a2, a2_off) = append_anchor(&fx.tape_service, &fx.sessions, &key, "A2").await;
    assert!(a1_off < a2_off, "A1 must precede A2 on disk");

    let err = fx
        .service
        // Swap them: from = A2 (later), to = A1 (earlier).
        .list_messages_between_anchors(&key, Some(a2), Some(a1))
        .await
        .expect_err("reversed anchors must error");

    assert!(
        err.to_string()
            .contains("from_anchor must precede to_anchor"),
        "error must explain the ordering rule, got: {err}"
    );
    assert!(
        matches!(err, ChatError::InvalidRequest { .. }),
        "must be an InvalidRequest variant",
    );
}
