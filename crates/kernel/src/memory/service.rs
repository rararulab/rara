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

//! Higher-level tape operations built on top of [`FileTapeStore`].
//!
//! `TapeService` is the main caller-facing API for session workflows. It
//! handles bootstrap anchors, fork/merge convenience flows, anchor-relative
//! queries, and search over persisted message entries.

use std::{future::Future, sync::Arc};

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use rapidfuzz::fuzz::RatioBatchComparator;
use serde_json::{Map, Value, json};
use snafu::ResultExt;
use unicode_normalization::UnicodeNormalization;

use super::{
    AnchorNode, AnchorSummary, AnchorTree, AppendOutcome, FileTapeStore, ForkEdge, HandoffState,
    SessionBranch, TapEntry, TapEntryKind, TapResult, get_fork_metadata,
};
use crate::{
    notification::{KernelNotification, NotificationBusRef},
    session::{AnchorRef, SessionDerivedState, SessionIndex, SessionIndexRef, SessionKey},
};

thread_local! {
    /// Per-thread current tape context used while executing fork closures.
    static TAPE_CONTEXT: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Read the per-turn correlation id from a tape entry's metadata JSON,
/// preferring the current key `rara_turn_id` and falling back to the
/// legacy `rara_message_id` key for tapes written before issue #1978.
///
/// Returns `None` when neither key is present or the value is not a
/// string.
pub fn read_turn_id(metadata: &Value) -> Option<&str> {
    metadata
        .get("rara_turn_id")
        .or_else(|| metadata.get("rara_message_id"))
        .and_then(|v| v.as_str())
}

/// Queries shorter than this skip fuzzy matching to avoid noisy results.
const MIN_FUZZY_QUERY_LENGTH: usize = 3;
/// Minimum normalized similarity ratio for a fuzzy hit.
const MIN_FUZZY_SCORE: f64 = 0.80;
/// Minimum number of query terms that should overlap before a partial hit is
/// considered relevant.
const MIN_QUERY_TERM_MATCHES: usize = 2;
/// Minimum query term coverage required for multi-term fallback matches.
const MIN_QUERY_TERM_COVERAGE: f64 = 0.60;
/// Exact full-query substring matches outrank all partial and fuzzy hits.
const EXACT_MATCH_BONUS: f64 = 1.0;

#[derive(Debug)]
struct SearchMatch {
    score:     f64,
    entry:     TapEntry,
    tape_name: String,
}

/// A single ranked hit returned by [`TapeService::search_across_tapes`].
///
/// Carries the tape (session) the entry belongs to alongside the entry
/// itself so cross-tape consumers (e.g. the admin session-search endpoint)
/// can attribute each match to its originating session without a second
/// round of lookups.
#[derive(Debug, Clone)]
pub struct TapeSearchHit {
    /// Matched tape entry.
    pub entry:     TapEntry,
    /// Name of the tape (typically the session key) the entry belongs to.
    pub tape_name: String,
}

/// Runtime tape info summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TapeInfo {
    /// Logical tape name.
    pub name: String,
    /// Total number of persisted entries.
    pub entries: usize,
    /// Total number of anchor entries.
    pub anchors: usize,
    /// Most recent anchor name, if any.
    pub last_anchor: Option<String>,
    /// Entries written after the most recent anchor.
    pub entries_since_last_anchor: usize,
    /// Last observed `total_tokens` usage from a `run` event.
    pub last_token_usage: Option<u64>,
    /// Estimated token count for the current context window (entries since
    /// last anchor).  Uses actual `prompt_tokens` from the most recent LLM
    /// call plus chars/4 estimates for entries added after that call.
    pub estimated_context_tokens: u64,
}

/// Get the current tape in contextual execution, mirroring Bub's
/// `current_tape`.
///
/// This is mainly useful while executing [`TapeService::fork_tape`], where the
/// closure temporarily runs against a forked tape context.
pub fn current_tape() -> String {
    TAPE_CONTEXT.with(|current| current.borrow().clone().unwrap_or_else(|| "-".to_owned()))
}

/// In-memory per-session derived-state cache used by [`TapeService`] to
/// maintain `SessionEntry` derived fields without rescanning the tape on
/// every append (issue #2025 — Decision 1 forbids `info()` calls on the
/// hot append path). Lazily populated on the first append per session
/// from the session-index row, then mutated incrementally and pushed
/// back to the index after every append.
#[derive(Debug, Clone, Default)]
struct DerivedCache {
    total_entries:             i64,
    entries_since_last_anchor: i64,
    last_token_usage:          Option<i64>,
    estimated_context_tokens:  i64,
    /// Sum of payload-string chars accumulated since the last LLM-usage
    /// entry, used by the `chars/4` estimator path (mirrors the
    /// `additional_chars` computation in [`TapeService::info`]).
    chars_since_last_usage:    u64,
    anchors:                   Vec<AnchorRef>,
    /// Whether this session already has a `preview` set in the index.
    /// `false` means the next user-role message should populate it.
    has_preview:               bool,
    /// Whether the cache has been hydrated from the index. `false`
    /// triggers a `get_session` on the next append.
    hydrated:                  bool,
}

/// Tape helper with app-specific operations.
///
/// Unlike the low-level [`FileTapeStore`], `TapeService` provides higher-level
/// workflows (anchors, fork/merge, search, LLM context building). It is **not**
/// bound to a specific tape — every method accepts a `tape_name` parameter so a
/// single instance can serve all sessions.
#[derive(Clone)]
pub struct TapeService {
    store:         FileTapeStore,
    fts:           Option<super::fts::TapeFts>,
    /// Optional notification bus for publishing tape mutation events.
    ///
    /// Wired in by `Kernel::new` after the bus is constructed, so external
    /// adapters can react to writes that happen outside a live user turn
    /// (background-task summaries, scheduled re-entries).
    notification:  Option<NotificationBusRef>,
    /// Optional session index — when wired, every successful append on a
    /// tape whose name parses as a [`SessionKey`] triggers a synchronous
    /// derived-state update. Issue #2025: the previous design left
    /// `SessionEntry::message_count` / `updated_at` as a stale snapshot
    /// from session-create time.
    session_index: Option<SessionIndexRef>,
    /// Per-session derived-state cache (see [`DerivedCache`]).
    derived_cache: Arc<DashMap<SessionKey, DerivedCache>>,
}

impl std::fmt::Debug for TapeService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TapeService")
            .field("store", &self.store)
            .field("fts", &self.fts.is_some())
            .field("notification", &self.notification.is_some())
            .field("session_index", &self.session_index.is_some())
            .field("derived_cache_len", &self.derived_cache.len())
            .finish()
    }
}

impl TapeService {
    /// Create a service backed by the given store.
    pub fn new(store: FileTapeStore) -> Self {
        Self {
            store,
            fts: None,
            notification: None,
            session_index: None,
            derived_cache: Arc::new(DashMap::new()),
        }
    }

    /// Create a service with FTS5 full-text search support.
    pub fn with_fts(
        store: FileTapeStore,
        pools: yunara_store::diesel_pool::DieselSqlitePools,
    ) -> Self {
        // Preload the jieba dictionary off the hot path. `warmup` is
        // idempotent — repeated `with_fts` calls do not leak threads.
        super::fts::warmup_tokenizer();
        Self {
            store,
            fts: Some(super::fts::TapeFts::new(pools)),
            notification: None,
            session_index: None,
            derived_cache: Arc::new(DashMap::new()),
        }
    }

    /// Attach a notification bus so message appends publish
    /// [`KernelNotification::TapeAppended`].
    #[must_use]
    pub fn with_notifications(mut self, bus: NotificationBusRef) -> Self {
        self.notification = Some(bus);
        self
    }

    /// Attach a [`SessionIndex`] so every append updates the owning
    /// session's derived-state row in band. Sessions whose tape name is
    /// not a [`SessionKey`] (user tapes, internal tapes) skip silently.
    #[must_use]
    pub fn with_session_index(mut self, index: SessionIndexRef) -> Self {
        self.session_index = Some(index);
        self
    }

    /// Access the underlying [`FileTapeStore`] for low-level operations such as
    /// fork/merge/discard that require direct store access.
    pub fn store(&self) -> &FileTapeStore { &self.store }

    /// Read all entries for the given tape.
    pub async fn entries(&self, tape_name: &str) -> TapResult<Vec<TapEntry>> {
        Ok(self.store.read(tape_name).await?.unwrap_or_default())
    }

    /// Load a specific entry by ID from a tape.
    pub async fn entry_by_id(&self, tape_name: &str, entry_id: u64) -> TapResult<Option<TapEntry>> {
        let entries = self.entries(tape_name).await?;
        Ok(entries.into_iter().find(|e| e.id == entry_id))
    }

    /// Load multiple entries by their IDs from a tape.
    pub async fn entries_by_ids(&self, tape_name: &str, ids: &[u64]) -> TapResult<Vec<TapEntry>> {
        let entries = self.entries(tape_name).await?;
        let id_set: std::collections::HashSet<u64> = ids.iter().copied().collect();
        Ok(entries
            .into_iter()
            .filter(|e| id_set.contains(&e.id))
            .collect())
    }

    /// Execute `func` against a forked tape. On success, merge the fork back
    /// into the parent tape. On failure, discard the fork so failed turns do
    /// not pollute the main tape.
    pub async fn fork_tape<T, F, Fut>(
        &self,
        tape_name: &str,
        at_entry_id: Option<u64>,
        func: F,
    ) -> TapResult<T>
    where
        F: FnOnce(String) -> Fut,
        Fut: Future<Output = TapResult<T>>,
    {
        let fork_name = self.store.fork(tape_name, at_entry_id).await?;

        let previous = TAPE_CONTEXT.with(|current| current.replace(Some(fork_name.clone())));
        let result = func(fork_name.clone()).await;
        TAPE_CONTEXT.with(|current| {
            current.replace(previous);
        });

        match &result {
            Ok(_) => self.store.merge(&fork_name, tape_name).await?,
            Err(_) => self.store.discard(&fork_name).await?,
        }
        result
    }

    /// Ensure the tape has an initial `session/start` anchor.
    pub async fn ensure_bootstrap_anchor(&self, tape_name: &str) -> TapResult<()> {
        if !self.anchors(tape_name, 1).await?.is_empty() {
            return Ok(());
        }
        let _ = self
            .handoff(
                tape_name,
                "session/start",
                HandoffState {
                    owner: Some("human".into()),
                    ..Default::default()
                },
            )
            .await?;
        Ok(())
    }

    /// Append an anchor and return entries from the most recent anchor onward.
    pub async fn handoff(
        &self,
        tape_name: &str,
        name: &str,
        state: HandoffState,
    ) -> TapResult<Vec<TapEntry>> {
        let outcome = self
            .store
            .append(
                tape_name,
                TapEntryKind::Anchor,
                json!({
                    "name": name,
                    "state": serde_json::to_value(&state).unwrap_or(Value::Object(Map::new())),
                }),
                None,
            )
            .await?;
        self.record_append(tape_name, &outcome).await;
        self.from_last_anchor(tape_name, None).await
    }

    /// Append an event entry.
    pub async fn append_event(&self, tape_name: &str, name: &str, data: Value) -> TapResult<()> {
        let outcome = self
            .store
            .append(
                tape_name,
                TapEntryKind::Event,
                json!({"name": name, "data": data}),
                None,
            )
            .await?;
        self.record_append(tape_name, &outcome).await;
        Ok(())
    }

    /// Append a system entry.
    pub async fn append_system(&self, tape_name: &str, content: &str) -> TapResult<()> {
        let outcome = self
            .store
            .append(
                tape_name,
                TapEntryKind::System,
                json!({"content": content}),
                None,
            )
            .await?;
        self.record_append(tape_name, &outcome).await;
        Ok(())
    }

    /// Append a message entry.
    pub async fn append_message(
        &self,
        tape_name: &str,
        payload: Value,
        metadata: Option<Value>,
    ) -> TapResult<TapEntry> {
        let outcome = self
            .store
            .append(tape_name, TapEntryKind::Message, payload, metadata)
            .await?;
        let entry = outcome.entry.clone();
        self.record_append(tape_name, &outcome).await;

        // Best-effort FTS indexing — errors are logged, never propagated.
        if let Some(fts) = &self.fts {
            if let Err(e) = fts
                .index_entries(tape_name, tape_name, std::slice::from_ref(&entry))
                .await
            {
                tracing::warn!(%e, tape_name, "FTS index failed on append");
            }
        }

        // Publish a tape-appended notification so adapters can refresh.
        // Best-effort: errors come from a parsed tape_name that does not
        // resolve to a SessionKey (user tape, internal tape) — those tapes
        // are not user-facing chat sessions and are intentionally skipped.
        if let Some(bus) = &self.notification {
            if let Ok(session_key) = SessionKey::try_from_raw(tape_name) {
                let role = entry
                    .payload
                    .get("role")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned);
                bus.publish(KernelNotification::TapeAppended {
                    session_key,
                    entry_id: entry.id,
                    role,
                    timestamp: entry.timestamp,
                })
                .await;
            }
        }

        Ok(entry)
    }

    /// Append a tool-call entry.
    pub async fn append_tool_call(
        &self,
        tape_name: &str,
        payload: Value,
        metadata: Option<Value>,
    ) -> TapResult<TapEntry> {
        let outcome = self
            .store
            .append(tape_name, TapEntryKind::ToolCall, payload, metadata)
            .await?;
        let entry = outcome.entry.clone();
        self.record_append(tape_name, &outcome).await;
        Ok(entry)
    }

    /// Append a tool-result entry.
    pub async fn append_tool_result(
        &self,
        tape_name: &str,
        payload: Value,
        metadata: Option<Value>,
    ) -> TapResult<TapEntry> {
        let outcome = self
            .store
            .append(tape_name, TapEntryKind::ToolResult, payload, metadata)
            .await?;
        let entry = outcome.entry.clone();
        self.record_append(tape_name, &outcome).await;
        Ok(entry)
    }

    /// Update the session-index row's tape-derived state after one append.
    ///
    /// Sessions whose tape name does not parse as a [`SessionKey`] (user
    /// tape, internal tape) skip silently. Errors loading the session row
    /// or writing the update are logged at warn level but never propagate
    /// — the tape JSONL is the source of truth and the boot reconciler
    /// (Decision 10) closes any drift.
    async fn record_append(&self, tape_name: &str, outcome: &AppendOutcome) {
        let Some(index) = &self.session_index else {
            return;
        };
        let Ok(session_key) = SessionKey::try_from_raw(tape_name) else {
            return;
        };

        // Hydrate the cache from the index on first encounter so we can
        // drive the per-session derived state forward without scanning
        // the tape on every append.
        {
            let entry = self.derived_cache.entry(session_key);
            let mut slot = entry.or_default();
            if !slot.hydrated {
                match index.get_session(&session_key).await {
                    Ok(Some(row)) => {
                        slot.total_entries = row.total_entries;
                        slot.entries_since_last_anchor = row.entries_since_last_anchor;
                        slot.last_token_usage = row.last_token_usage;
                        slot.estimated_context_tokens = row.estimated_context_tokens;
                        slot.anchors = row.anchors;
                        slot.has_preview = row.preview.is_some();
                        slot.chars_since_last_usage = 0;
                    }
                    Ok(None) => {
                        // Row not in the index yet — `record_append` may
                        // race a not-yet-committed `create_session`. The
                        // cache stays at default; the next append after
                        // creation will pick it up via the hydrate path.
                    }
                    Err(e) => {
                        tracing::warn!(
                            %e, %session_key,
                            "session-index hydrate failed on append; deferring"
                        );
                        return;
                    }
                }
                slot.hydrated = true;
            }
        }

        // Compute the post-append derived state and the preview to set.
        // Cloned out so the DashMap slot is released before the await.
        let (derived, preview_to_set) = {
            let mut slot = self
                .derived_cache
                .get_mut(&session_key)
                .expect("cache entry created above");

            slot.total_entries = outcome.total_entries_after;
            let entry_ts: DateTime<Utc> = jiff_to_chrono(outcome.entry.timestamp);

            // -- per-kind incremental updates -----------------------------
            match outcome.entry.kind {
                TapEntryKind::Anchor => {
                    let segment = slot.entries_since_last_anchor + 1;
                    let name = outcome
                        .entry
                        .payload
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("-")
                        .to_owned();
                    slot.anchors.push(AnchorRef {
                        anchor_id: outcome.entry.id,
                        byte_offset: outcome.byte_offset,
                        name,
                        timestamp: entry_ts,
                        entry_count_in_segment: segment,
                    });
                    slot.entries_since_last_anchor = 0;
                    slot.estimated_context_tokens = 0;
                    slot.chars_since_last_usage = 0;
                }
                _ => {
                    slot.entries_since_last_anchor += 1;
                }
            }

            // Track usage from `llm.run` events and from
            // `usage.prompt_tokens` carried on assistant message metadata.
            if outcome.entry.kind == TapEntryKind::Event {
                let event_name = outcome.entry.payload.get("name").and_then(Value::as_str);
                if matches!(event_name, Some("run" | "llm.run")) {
                    if let Some(total) = outcome
                        .entry
                        .payload
                        .get("data")
                        .and_then(Value::as_object)
                        .and_then(|d| d.get("usage"))
                        .and_then(Value::as_object)
                        .and_then(|u| u.get("total_tokens"))
                        .and_then(Value::as_u64)
                    {
                        slot.last_token_usage = Some(total as i64);
                    }
                }
            }
            if let Some(meta) = outcome.entry.metadata.as_ref() {
                if let Some(prompt) = meta
                    .get("usage")
                    .and_then(|u| u.get("prompt_tokens"))
                    .and_then(Value::as_u64)
                {
                    let completion = meta
                        .get("usage")
                        .and_then(|u| u.get("completion_tokens"))
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    slot.estimated_context_tokens = (prompt + completion) as i64;
                    slot.chars_since_last_usage = 0;
                }
            }
            // For conversational entries that don't carry usage, fall back
            // to the `chars/4` estimator (mirrors `TapeService::info`).
            if matches!(
                outcome.entry.kind,
                TapEntryKind::Message | TapEntryKind::ToolCall | TapEntryKind::ToolResult
            ) && outcome
                .entry
                .metadata
                .as_ref()
                .and_then(|m| m.get("usage"))
                .is_none()
            {
                slot.chars_since_last_usage = slot
                    .chars_since_last_usage
                    .saturating_add(outcome.entry.payload.to_string().len() as u64);
                slot.estimated_context_tokens = slot
                    .estimated_context_tokens
                    .saturating_add((slot.chars_since_last_usage as i64) / 4);
            }

            // First user-role message becomes the preview.
            let preview = if !slot.has_preview
                && outcome.entry.kind == TapEntryKind::Message
                && outcome.entry.payload.get("role").and_then(Value::as_str) == Some("user")
            {
                let text = extract_message_preview_text(&outcome.entry.payload);
                if let Some(p) = text {
                    slot.has_preview = true;
                    Some(p)
                } else {
                    None
                }
            } else {
                None
            };

            let derived = SessionDerivedState::builder()
                .total_entries(slot.total_entries)
                .updated_at(entry_ts)
                .maybe_last_token_usage(slot.last_token_usage)
                .estimated_context_tokens(slot.estimated_context_tokens)
                .entries_since_last_anchor(slot.entries_since_last_anchor)
                .anchors(slot.anchors.clone())
                .maybe_preview(preview.clone())
                .build();
            (derived, preview)
        };

        if let Err(e) = index.update_session_derived(&session_key, &derived).await {
            tracing::warn!(
                %e, %session_key,
                "session-index derived-state update failed; \
                 boot reconciler will repair on next start"
            );
        }
        let _ = preview_to_set;
    }

    /// Build LLM-ready messages from tape entries since the last anchor.
    ///
    /// If the last anchor carries a `summary` or `next_steps` in its state,
    /// a system message is injected so the LLM retains key context from
    /// before the anchor even after older entries leave the context window.
    pub async fn build_llm_context(&self, tape_name: &str) -> TapResult<Vec<crate::llm::Message>> {
        // Load all entry kinds so we can inspect the anchor itself.
        let all_entries = self.from_last_anchor(tape_name, None).await?;

        // Filter to conversational kinds for LLM message reconstruction.
        let conv_entries: Vec<_> = all_entries
            .iter()
            .filter(|e| {
                matches!(
                    e.kind,
                    TapEntryKind::Message | TapEntryKind::ToolCall | TapEntryKind::ToolResult
                )
            })
            .cloned()
            .collect();
        let mut messages = super::context::default_tape_context(&conv_entries)?;

        // Inject anchor state (summary/next_steps) as a system message.
        if let Some(anchor_msg) = super::context::anchor_context(&all_entries) {
            let insert_pos = messages
                .iter()
                .position(|m| m.role != crate::llm::Role::System)
                .unwrap_or(messages.len());
            messages.insert(insert_pos, anchor_msg);
        }

        Ok(messages)
    }

    /// Build LLM-ready messages from a session tape, prepending user-specific
    /// context from the corresponding user tape when available.
    ///
    /// This is the primary entry point for context construction when a user
    /// identity is known.  The user tape is loaded once per turn and injected
    /// as a system message at the front of the message list so the LLM sees
    /// accumulated user knowledge before the conversation history.
    pub async fn build_llm_context_with_user(
        &self,
        tape_name: &str,
        user_id: &str,
    ) -> TapResult<Vec<crate::llm::Message>> {
        let mut messages = self.build_llm_context(tape_name).await?;

        // Load user tape entries since the last anchor so accumulated notes
        // respect distillation boundaries.  The anchor summary (if any)
        // carries previously-distilled knowledge.
        let user_tape = super::user_tape_name(user_id);
        let user_entries = self.from_last_anchor(&user_tape, None).await?;
        let anchor_summary = super::context::anchor_summary_from_entries(&user_entries);
        if let Some(user_msg) =
            super::context::user_tape_context(&user_entries, anchor_summary.as_deref())
        {
            let insert_pos = messages
                .iter()
                .position(|m| m.role != crate::llm::Role::System)
                .unwrap_or(messages.len());
            messages.insert(insert_pos, user_msg);
        }

        Ok(messages)
    }

    /// Rebuild the complete LLM message list from tape.
    ///
    /// This is the single source of truth for what the LLM sees:
    /// 1. System prompt (effective_prompt)
    /// 2. Anchor context (if any)
    /// 3. User memory context (if any)
    /// 4. Conversation history since last anchor
    ///
    /// Called at the start of each agent loop iteration instead of
    /// maintaining an in-memory messages vector.
    #[tracing::instrument(skip(self, system_prompt))]
    pub async fn rebuild_messages_for_llm(
        &self,
        tape_name: &str,
        user_id: Option<&str>,
        system_prompt: &str,
    ) -> TapResult<Vec<crate::llm::Message>> {
        let mut messages = vec![crate::llm::Message::system(system_prompt)];

        let history = match user_id {
            Some(uid) => self.build_llm_context_with_user(tape_name, uid).await?,
            None => self.build_llm_context(tape_name).await?,
        };
        messages.extend(history);

        // Collapse every `system` role into position 0: merge leading system
        // messages and rewrite any mid-stream `system` message as a prefixed
        // `user` turn. This is mandatory for providers that reject non-first
        // system roles (MiniMax `invalid message role: system (2013)`) and
        // safe for providers that tolerate them.
        Ok(super::context::collapse_system_messages(messages))
    }

    // -----------------------------------------------------------------------
    // User tape helpers
    // -----------------------------------------------------------------------

    /// Append a structured note to a user tape.
    ///
    /// Notes are the primary entry kind for user tapes. Each note carries a
    /// `category` tag (e.g. `"preference"`, `"fact"`, `"todo"`) and free-form
    /// `content`.
    pub async fn append_user_note(
        &self,
        user_id: &str,
        category: &str,
        content: &str,
    ) -> TapResult<TapEntry> {
        let user_tape = super::user_tape_name(user_id);
        let outcome = self
            .store
            .append(
                &user_tape,
                TapEntryKind::Note,
                serde_json::json!({
                    "category": category,
                    "content": content,
                }),
                None,
            )
            .await?;
        // User tapes have no SessionEntry to update — `record_append`
        // will short-circuit on the SessionKey parse — but we still call
        // it for symmetry / future-proofing.
        self.record_append(&user_tape, &outcome).await;
        Ok(outcome.entry)
    }

    /// Read all note entries from a user tape.
    pub async fn read_user_notes(&self, user_id: &str) -> TapResult<Vec<TapEntry>> {
        let user_tape = super::user_tape_name(user_id);
        // Use the store's kind index so this is O(k) in the number of notes
        // rather than O(n) over the whole user tape.
        self.store
            .entries_by_kind(&user_tape, TapEntryKind::Note)
            .await
    }

    /// Inspect current tape state without mutating it.
    pub async fn info(&self, tape_name: &str) -> TapResult<TapeInfo> {
        let entries = self.entries(tape_name).await?;
        // Pull the anchor list once via the kind index, then borrow into it
        // for the remainder of the function so the linear `filter` calls
        // disappear.
        let anchor_entries: Vec<&TapEntry> = entries
            .iter()
            .filter(|entry| entry.kind == TapEntryKind::Anchor)
            .collect();
        let last_anchor = anchor_entries
            .last()
            .and_then(|entry| entry.payload.get("name"))
            .and_then(Value::as_str)
            .map(str::to_owned);

        let entries_since_last_anchor = if let Some(last) = anchor_entries.last() {
            entries.iter().filter(|entry| entry.id > last.id).count()
        } else {
            entries.len()
        };

        let last_token_usage = entries.iter().rev().find_map(|entry| {
            if entry.kind != TapEntryKind::Event {
                return None;
            }
            let event_name = entry.payload.get("name").and_then(Value::as_str);
            if !matches!(event_name, Some("run" | "llm.run")) {
                return None;
            }
            entry
                .payload
                .get("data")
                .and_then(Value::as_object)
                .and_then(|data| data.get("usage"))
                .and_then(Value::as_object)
                .and_then(|usage| usage.get("total_tokens"))
                .and_then(Value::as_u64)
        });

        // Compute estimated context tokens for entries since last anchor.
        let anchor_id = anchor_entries.last().map(|a| a.id).unwrap_or(0);
        let since_anchor: Vec<&TapEntry> = entries.iter().filter(|e| e.id > anchor_id).collect();

        // Find the last assistant entry with usage metadata.
        let mut last_known_tokens: u64 = 0;
        let mut last_usage_entry_id: u64 = 0;
        for entry in since_anchor.iter().rev() {
            if let Some(meta) = &entry.metadata {
                if let Some(prompt_tokens) = meta
                    .get("usage")
                    .and_then(|u| u.get("prompt_tokens"))
                    .and_then(Value::as_u64)
                {
                    last_known_tokens = prompt_tokens;
                    last_usage_entry_id = entry.id;
                    // Also add this entry's own completion tokens
                    let completion = meta
                        .get("usage")
                        .and_then(|u| u.get("completion_tokens"))
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    last_known_tokens += completion;
                    break;
                }
            }
        }

        // Estimate tokens for entries added after the last usage-bearing entry.
        let additional_chars: usize = since_anchor
            .iter()
            .filter(|e| e.id > last_usage_entry_id)
            .filter(|e| {
                matches!(
                    e.kind,
                    TapEntryKind::Message | TapEntryKind::ToolCall | TapEntryKind::ToolResult
                )
            })
            .map(|e| e.payload.to_string().len())
            .sum();
        let estimated_context_tokens = last_known_tokens + (additional_chars as u64 / 4);

        Ok(TapeInfo {
            name: tape_name.to_owned(),
            entries: entries.len(),
            anchors: anchor_entries.len(),
            last_anchor,
            entries_since_last_anchor,
            last_token_usage,
            estimated_context_tokens,
        })
    }

    /// Reset the tape, optionally archiving the previous file first.
    pub async fn reset(&self, tape_name: &str, archive: bool) -> TapResult<String> {
        let archive_path = if archive {
            self.store.archive(tape_name).await?
        } else {
            None
        };

        self.store.reset(tape_name).await?;

        // Clean up derived FTS index so stale rows don't survive reset.
        if let Some(fts) = &self.fts {
            if let Err(e) = fts.remove_tape(tape_name).await {
                tracing::warn!(%e, tape_name, "FTS cleanup failed on reset");
            }
        }

        let handoff_state = HandoffState {
            owner: Some("human".into()),
            extra: archive_path
                .as_ref()
                .map(|path| json!({ "archived": path.to_string_lossy() })),
            ..Default::default()
        };
        let _ = self
            .handoff(tape_name, "session/start", handoff_state)
            .await?;

        Ok(if let Some(path) = archive_path {
            format!("Archived: {}", path.display())
        } else {
            "ok".to_owned()
        })
    }

    /// Delete the tape file for a session permanently.
    ///
    /// Unlike [`Self::reset`], this does not create a new bootstrap anchor —
    /// the tape is simply removed. Used when deleting a session entirely.
    pub async fn delete_tape(&self, tape_name: &str) -> TapResult<()> {
        self.store.reset(tape_name).await?;

        if let Some(fts) = &self.fts {
            if let Err(e) = fts.remove_tape(tape_name).await {
                tracing::warn!(%e, tape_name, "FTS cleanup failed on delete");
            }
        }

        Ok(())
    }

    /// Create a new tape at `target` containing all entries from `source`
    /// up to and including the anchor named `anchor_name`.
    ///
    /// This is the kernel-level primitive for session forking from an anchor
    /// checkpoint. The caller is responsible for session metadata creation.
    pub async fn checkout_anchor(
        &self,
        source: &str,
        anchor_name: &str,
        target: &str,
    ) -> TapResult<()> {
        // O(1) lookup via the anchor-name index instead of a reverse linear
        // scan over every cached entry on the source tape.
        let anchor_id = self
            .store
            .last_anchor_id_by_name(source, anchor_name)
            .await?
            .ok_or_else(|| super::TapError::State {
                message: format!("anchor not found: {anchor_name}"),
            })?;

        let entries = self.entries(source).await?;

        for entry in entries.iter().filter(|e| e.id <= anchor_id) {
            self.store
                .append(
                    target,
                    entry.kind,
                    entry.payload.clone(),
                    entry.metadata.clone(),
                )
                .await?;
        }

        Ok(())
    }

    /// Return the most recent `limit` anchors, oldest-to-newest within the
    /// returned window.
    pub async fn anchors(&self, tape_name: &str, limit: usize) -> TapResult<Vec<AnchorSummary>> {
        // O(k) over the anchor entries via the kind index.
        let anchor_entries = self
            .store
            .entries_by_kind(tape_name, TapEntryKind::Anchor)
            .await?;
        let start = anchor_entries.len().saturating_sub(limit);
        Ok(anchor_entries[start..]
            .iter()
            .map(|entry| {
                let name = entry
                    .payload
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("-")
                    .to_owned();
                let state = entry
                    .payload
                    .get("state")
                    .cloned()
                    .filter(|value| value.is_object())
                    .unwrap_or_else(|| Value::Object(Map::new()));
                AnchorSummary { name, state }
            })
            .collect())
    }

    /// Return entries strictly between two named anchors.
    pub async fn between_anchors(
        &self,
        tape_name: &str,
        start: &str,
        end: &str,
        kinds: Option<&[TapEntryKind]>,
    ) -> TapResult<Vec<TapEntry>> {
        // Resolve both anchor IDs via the index, then load entries once.
        let start_id = self.store.last_anchor_id_by_name(tape_name, start).await?;
        let end_id = self.store.last_anchor_id_by_name(tape_name, end).await?;
        let entries = self.entries(tape_name).await?;
        Ok(entries
            .into_iter()
            .filter(|entry| start_id.is_some_and(|id| entry.id > id))
            .filter(|entry| end_id.is_some_and(|id| entry.id < id))
            .filter(|entry| kind_matches(entry, kinds))
            .collect())
    }

    /// Return entries after the most recent anchor named `anchor`.
    pub async fn after_anchor(
        &self,
        tape_name: &str,
        anchor: &str,
        kinds: Option<&[TapEntryKind]>,
    ) -> TapResult<Vec<TapEntry>> {
        let anchor_id = self.store.last_anchor_id_by_name(tape_name, anchor).await?;
        let entries = self.entries(tape_name).await?;
        Ok(entries
            .into_iter()
            .filter(|entry| anchor_id.is_none_or(|id| entry.id > id))
            .filter(|entry| kind_matches(entry, kinds))
            .collect())
    }

    /// Return entries from the most recent anchor onward.
    pub async fn from_last_anchor(
        &self,
        tape_name: &str,
        kinds: Option<&[TapEntryKind]>,
    ) -> TapResult<Vec<TapEntry>> {
        // O(1) anchor lookup via the store's kind index, instead of a full
        // reverse scan over every cached entry.
        let last_anchor_id = self.store.last_anchor_id(tape_name).await?;
        let entries = self.entries(tape_name).await?;
        Ok(entries
            .into_iter()
            .filter(|entry| last_anchor_id.is_none_or(|id| entry.id >= id))
            .filter(|entry| kind_matches(entry, kinds))
            .collect())
    }

    /// Return entries whose ID is strictly greater than `after_entry_id`.
    ///
    /// Used by the context folding cooldown logic to count entries added since
    /// the last auto-fold anchor.
    pub async fn entries_after(
        &self,
        tape_name: &str,
        after_entry_id: u64,
    ) -> TapResult<Vec<TapEntry>> {
        let entries = self.entries(tape_name).await?;
        Ok(entries
            .into_iter()
            .filter(|e| e.id > after_entry_id)
            .collect())
    }

    /// Return the ID of the last entry on the tape, or 0 if the tape is empty.
    pub async fn last_entry_id(&self, tape_name: &str) -> TapResult<u64> {
        let entries = self.entries(tape_name).await?;
        Ok(entries.last().map(|e| e.id).unwrap_or(0))
    }

    /// Find all tape entries (any kind) whose `metadata.rara_turn_id`
    /// matches the given ID. Unlike [`Self::search`], this is an exact
    /// metadata filter — no text ranking, no kind restriction.
    ///
    /// Used by the `/debug` command to retrieve the full execution context
    /// (messages + tool calls + tool results) for a single turn.
    ///
    /// Reads both `rara_turn_id` (current key) and `rara_message_id`
    /// (legacy key, present in tape JSONL files written before issue
    /// #1978). New writers only emit the new key, but existing on-disk
    /// tapes are append-only and must remain readable.
    pub async fn entries_by_turn_id(
        &self,
        tape_name: &str,
        turn_id: &str,
    ) -> TapResult<Vec<TapEntry>> {
        let entries = self.store.read(tape_name).await?.unwrap_or_default();
        Ok(entries
            .into_iter()
            .filter(|e| {
                e.metadata
                    .as_ref()
                    .and_then(read_turn_id)
                    .is_some_and(|id| id == turn_id)
            })
            .collect())
    }

    /// Search message entries using ranked Unicode-aware text matching.
    ///
    /// When FTS5 is available, uses it for candidate retrieval then re-ranks
    /// with the existing scorer.  Falls back to brute-force on FTS error.
    pub async fn search(
        &self,
        tape_name: &str,
        query: &str,
        limit: usize,
        all_tapes: bool,
    ) -> TapResult<Vec<TapEntry>> {
        let normalized_query = normalize_search_text(query);
        if normalized_query.is_empty() {
            return Ok(Vec::new());
        }

        // Try FTS candidate retrieval first.
        if let Some(fts) = &self.fts {
            let tape_filter = if all_tapes { None } else { Some(tape_name) };

            // Lazy backfill: ensure all tapes are indexed before querying.
            if let Err(e) = self.backfill_fts(fts, tape_name, all_tapes).await {
                tracing::warn!(%e, "FTS backfill failed, falling back to brute-force");
            } else {
                // Fetch 3x candidates so re-ranking has room to filter.
                match fts.search(query, tape_filter, limit * 3).await {
                    Ok(hits) if !hits.is_empty() => {
                        return self.rerank_fts_hits(&hits, &normalized_query, limit).await;
                    }
                    Err(e) => {
                        tracing::warn!(%e, "FTS search failed, falling back to brute-force");
                    }
                    _ => {} // empty hits — fall through to brute-force
                }
            }
        }

        // Brute-force fallback (original path).
        self.search_brute_force(tape_name, &normalized_query, limit, all_tapes)
            .await
    }

    /// Brute-force search over all entries (original algorithm).
    async fn search_brute_force(
        &self,
        tape_name: &str,
        normalized_query: &str,
        limit: usize,
        all_tapes: bool,
    ) -> TapResult<Vec<TapEntry>> {
        let query_terms = extract_query_terms(normalized_query);
        let query_scorer = (normalized_query.chars().count() >= MIN_FUZZY_QUERY_LENGTH)
            .then(|| RatioBatchComparator::new(normalized_query.chars()));

        let tape_names = if all_tapes {
            self.store.list_tapes().await?
        } else {
            vec![tape_name.to_owned()]
        };

        let mut results = Vec::new();
        for name in tape_names {
            let entries = self.store.read(&name).await?.unwrap_or_default();
            for entry in entries.into_iter().rev() {
                if entry.kind != TapEntryKind::Message {
                    continue;
                }
                let searchable_text = normalize_search_text(&extract_searchable_text(
                    &entry.payload,
                    entry.metadata.as_ref(),
                ));
                let Some(score) = score_search_candidate(
                    normalized_query,
                    &query_terms,
                    &searchable_text,
                    query_scorer.as_ref(),
                ) else {
                    continue;
                };
                results.push(SearchMatch {
                    score,
                    entry,
                    tape_name: name.clone(),
                });
            }
        }

        results.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| right.entry.id.cmp(&left.entry.id))
        });
        results.truncate(limit);

        Ok(results.into_iter().map(|item| item.entry).collect())
    }

    /// Backfill FTS index for tapes that have un-indexed entries.
    async fn backfill_fts(
        &self,
        fts: &super::fts::TapeFts,
        tape_name: &str,
        all_tapes: bool,
    ) -> crate::error::Result<()> {
        let tape_names = if all_tapes {
            self.store.list_tapes().await.unwrap_or_default()
        } else {
            vec![tape_name.to_owned()]
        };

        for name in &tape_names {
            let entries = self
                .store
                .read(name)
                .await
                .unwrap_or_default()
                .unwrap_or_default();
            if entries.is_empty() {
                continue;
            }
            fts.index_entries(name, name, &entries).await?;
        }
        Ok(())
    }

    /// Load full entries for FTS hits and re-rank with the existing scorer.
    async fn rerank_fts_hits(
        &self,
        hits: &[super::fts::FtsHit],
        normalized_query: &str,
        limit: usize,
    ) -> TapResult<Vec<TapEntry>> {
        let query_terms = extract_query_terms(normalized_query);
        let query_scorer = (normalized_query.chars().count() >= MIN_FUZZY_QUERY_LENGTH)
            .then(|| RatioBatchComparator::new(normalized_query.chars()));

        // Collect entry IDs grouped by tape for batch loading.
        let mut by_tape: std::collections::HashMap<&str, Vec<u64>> =
            std::collections::HashMap::new();
        for hit in hits {
            by_tape
                .entry(&hit.tape_name)
                .or_default()
                .push(hit.entry_id);
        }

        let mut results = Vec::new();
        for (tape, ids) in &by_tape {
            let entries = self.store.read(tape).await?.unwrap_or_default();
            let id_set: std::collections::HashSet<u64> = ids.iter().copied().collect();
            for entry in entries {
                if !id_set.contains(&entry.id) {
                    continue;
                }
                let searchable_text = normalize_search_text(&extract_searchable_text(
                    &entry.payload,
                    entry.metadata.as_ref(),
                ));
                let Some(score) = score_search_candidate(
                    normalized_query,
                    &query_terms,
                    &searchable_text,
                    query_scorer.as_ref(),
                ) else {
                    continue;
                };
                results.push(SearchMatch {
                    score,
                    entry,
                    tape_name: (*tape).to_owned(),
                });
            }
        }

        results.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| right.entry.id.cmp(&left.entry.id))
        });
        results.truncate(limit);

        Ok(results.into_iter().map(|item| item.entry).collect())
    }

    /// Cross-tape ranked search that preserves tape attribution.
    ///
    /// Equivalent to [`Self::search`] with `all_tapes = true`, except each
    /// hit carries the originating tape name. When FTS5 is available, this
    /// issues a **single** index query (the tape_name column is already
    /// tracked in `FtsHit`, so attribution is free); otherwise it falls
    /// back to a brute-force scan across all tapes. The underlying scoring
    /// is identical to `search` so ordering stays stable.
    pub async fn search_across_tapes(
        &self,
        query: &str,
        limit: usize,
    ) -> TapResult<Vec<TapeSearchHit>> {
        let normalized_query = normalize_search_text(query);
        if normalized_query.is_empty() {
            return Ok(Vec::new());
        }

        if let Some(fts) = &self.fts {
            // Lazy backfill across all tapes so newly-added entries are
            // visible to the index before the single query fires.
            if let Err(e) = self.backfill_fts(fts, "", true).await {
                tracing::warn!(%e, "FTS backfill failed, falling back to brute-force");
            } else {
                match fts.search(query, None, limit * 3).await {
                    Ok(hits) if !hits.is_empty() => {
                        return self
                            .rerank_fts_hits_with_tape(&hits, &normalized_query, limit)
                            .await;
                    }
                    Err(e) => {
                        tracing::warn!(%e, "FTS search failed, falling back to brute-force");
                    }
                    _ => {}
                }
            }
        }

        self.search_brute_force_with_tape(&normalized_query, limit)
            .await
    }

    /// Brute-force variant of [`Self::search_across_tapes`] — retains the
    /// tape name for each match. Only invoked when FTS is unavailable or
    /// returned an error.
    async fn search_brute_force_with_tape(
        &self,
        normalized_query: &str,
        limit: usize,
    ) -> TapResult<Vec<TapeSearchHit>> {
        let query_terms = extract_query_terms(normalized_query);
        let query_scorer = (normalized_query.chars().count() >= MIN_FUZZY_QUERY_LENGTH)
            .then(|| RatioBatchComparator::new(normalized_query.chars()));

        let tape_names = self.store.list_tapes().await?;
        let mut results = Vec::new();
        for name in tape_names {
            let entries = self.store.read(&name).await?.unwrap_or_default();
            for entry in entries.into_iter().rev() {
                if entry.kind != TapEntryKind::Message {
                    continue;
                }
                let searchable_text = normalize_search_text(&extract_searchable_text(
                    &entry.payload,
                    entry.metadata.as_ref(),
                ));
                let Some(score) = score_search_candidate(
                    normalized_query,
                    &query_terms,
                    &searchable_text,
                    query_scorer.as_ref(),
                ) else {
                    continue;
                };
                results.push(SearchMatch {
                    score,
                    entry,
                    tape_name: name.clone(),
                });
            }
        }

        results.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| right.entry.id.cmp(&left.entry.id))
        });
        results.truncate(limit);

        Ok(results
            .into_iter()
            .map(|item| TapeSearchHit {
                entry:     item.entry,
                tape_name: item.tape_name,
            })
            .collect())
    }

    /// Re-rank FTS candidates while preserving the originating tape name.
    async fn rerank_fts_hits_with_tape(
        &self,
        hits: &[super::fts::FtsHit],
        normalized_query: &str,
        limit: usize,
    ) -> TapResult<Vec<TapeSearchHit>> {
        let query_terms = extract_query_terms(normalized_query);
        let query_scorer = (normalized_query.chars().count() >= MIN_FUZZY_QUERY_LENGTH)
            .then(|| RatioBatchComparator::new(normalized_query.chars()));

        let mut by_tape: std::collections::HashMap<&str, Vec<u64>> =
            std::collections::HashMap::new();
        for hit in hits {
            by_tape
                .entry(&hit.tape_name)
                .or_default()
                .push(hit.entry_id);
        }

        let mut results = Vec::new();
        for (tape, ids) in &by_tape {
            let entries = self.store.read(tape).await?.unwrap_or_default();
            let id_set: std::collections::HashSet<u64> = ids.iter().copied().collect();
            for entry in entries {
                if !id_set.contains(&entry.id) {
                    continue;
                }
                let searchable_text = normalize_search_text(&extract_searchable_text(
                    &entry.payload,
                    entry.metadata.as_ref(),
                ));
                let Some(score) = score_search_candidate(
                    normalized_query,
                    &query_terms,
                    &searchable_text,
                    query_scorer.as_ref(),
                ) else {
                    continue;
                };
                results.push(SearchMatch {
                    score,
                    entry,
                    tape_name: (*tape).to_owned(),
                });
            }
        }

        results.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| right.entry.id.cmp(&left.entry.id))
        });
        results.truncate(limit);

        Ok(results
            .into_iter()
            .map(|item| TapeSearchHit {
                entry:     item.entry,
                tape_name: item.tape_name,
            })
            .collect())
    }

    /// List all tape names known to the underlying store.
    pub async fn list_tapes(&self) -> TapResult<Vec<String>> { self.store.list_tapes().await }

    /// Build the anchor tree for `session_key`, rooted at its oldest ancestor.
    pub async fn build_anchor_tree(
        &self,
        session_key: &str,
        sessions: &dyn SessionIndex,
    ) -> TapResult<AnchorTree> {
        // Always render from the original ancestor so the graph is stable no
        // matter which forked session the user is currently in.
        let root_key = self.find_root_session(session_key, sessions).await?;
        // TODO(perf): walk only the fork chain instead of loading all sessions.
        // This is O(all_sessions) but anchor trees are rarely deep, so acceptable for
        // now.
        let all_sessions = sessions
            .list_sessions(10_000, 0)
            .await
            .context(super::error::SessionSnafu)?;

        let mut sessions_by_key = std::collections::HashMap::new();
        let mut fork_index: std::collections::HashMap<String, Vec<(String, String)>> =
            std::collections::HashMap::new();

        for entry in all_sessions {
            let key = entry.key.to_string();
            // Build parent -> (anchor, child) index from metadata so we can
            // attach fork branches in one recursive pass.
            if let Some(fm) = get_fork_metadata(&entry.metadata) {
                fork_index
                    .entry(fm.forked_from)
                    .or_default()
                    .push((fm.forked_at_anchor, key.clone()));
            }
            sessions_by_key.insert(key, entry);
        }

        if !sessions_by_key.contains_key(&root_key) {
            return Err(super::TapError::State {
                message: format!("root session not found: {root_key}"),
            });
        }

        let mut anchors_by_key = std::collections::HashMap::new();
        for key in sessions_by_key.keys() {
            // Preload all branch anchor nodes up front. This keeps recursive
            // branch assembly synchronous and deterministic.
            anchors_by_key.insert(key.clone(), self.load_anchor_nodes(key).await?);
        }

        let root = build_session_branch(
            &root_key,
            &sessions_by_key,
            &anchors_by_key,
            &fork_index,
            &mut std::collections::HashSet::new(),
        )?;

        Ok(AnchorTree {
            root,
            current_session: session_key.to_owned(),
        })
    }

    pub(crate) async fn find_root_session(
        &self,
        session_key: &str,
        sessions: &dyn SessionIndex,
    ) -> TapResult<String> {
        let mut current = session_key.to_owned();
        let mut visited = std::collections::HashSet::new();

        loop {
            // Defend against corrupted metadata chains.
            if !visited.insert(current.clone()) {
                return Err(super::TapError::State {
                    message: format!("cycle detected in fork metadata at session: {current}"),
                });
            }

            let key = SessionKey::try_from_raw(&current).map_err(|e| super::TapError::State {
                message: format!("invalid session key while resolving root: {current} ({e})"),
            })?;
            let Some(entry) = sessions
                .get_session(&key)
                .await
                .context(super::error::SessionSnafu)?
            else {
                break;
            };
            let Some(fm) = get_fork_metadata(&entry.metadata) else {
                break;
            };
            current = fm.forked_from;
        }

        Ok(current)
    }

    async fn load_anchor_nodes(&self, session_key: &str) -> TapResult<Vec<AnchorNode>> {
        // Use the kind index so this is O(k) in the number of anchor entries
        // rather than O(n) over the whole tape.
        let entries = self
            .store
            .entries_by_kind(session_key, TapEntryKind::Anchor)
            .await?;
        Ok(entries
            .into_iter()
            .map(|entry| {
                // Be permissive with malformed payloads to avoid dropping the
                // entire tree for one bad anchor record.
                let name = entry
                    .payload
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("-")
                    .to_owned();
                let summary = entry
                    .payload
                    .get("state")
                    .and_then(|state| state.get("summary"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                AnchorNode {
                    name,
                    summary,
                    entry_id: entry.id,
                }
            })
            .collect())
    }
}

fn build_session_branch(
    session_key: &str,
    sessions_by_key: &std::collections::HashMap<String, crate::session::SessionEntry>,
    anchors_by_key: &std::collections::HashMap<String, Vec<AnchorNode>>,
    fork_index: &std::collections::HashMap<String, Vec<(String, String)>>,
    visited: &mut std::collections::HashSet<String>,
) -> TapResult<SessionBranch> {
    if !visited.insert(session_key.to_owned()) {
        return Err(super::TapError::State {
            message: format!("cycle detected while building tree at session: {session_key}"),
        });
    }

    let session_entry = sessions_by_key
        .get(session_key)
        .ok_or_else(|| super::TapError::State {
            message: format!("session not found while building tree: {session_key}"),
        })?;

    let mut forks = Vec::new();
    if let Some(children) = fork_index.get(session_key) {
        let mut ordered = children.clone();
        // Keep output ordering stable for deterministic snapshots/tests.
        ordered.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        for (at_anchor, child_key) in ordered {
            let child_branch = build_session_branch(
                &child_key,
                sessions_by_key,
                anchors_by_key,
                fork_index,
                visited,
            )?;
            forks.push(ForkEdge {
                at_anchor,
                branch: child_branch,
            });
        }
    }

    visited.remove(session_key);

    Ok(SessionBranch {
        session_key: session_key.to_owned(),
        title: session_entry.title.clone(),
        anchors: anchors_by_key.get(session_key).cloned().unwrap_or_default(),
        forks,
    })
}

/// Apply an optional kind filter to one entry.
fn kind_matches(entry: &TapEntry, kinds: Option<&[TapEntryKind]>) -> bool {
    kinds.is_none_or(|kinds| kinds.iter().any(|kind| kind == &entry.kind))
}

/// Convert a `jiff::Timestamp` to `chrono::DateTime<Utc>`.
///
/// Falls back to the current wall clock if conversion fails (only
/// possible for timestamps outside the chrono representable range,
/// which is not reachable for any real tape entry).
fn jiff_to_chrono(ts: jiff::Timestamp) -> chrono::DateTime<chrono::Utc> {
    let mut second = ts.as_second();
    let mut nanosecond = ts.subsec_nanosecond();
    if nanosecond < 0 {
        second = second.saturating_sub(1);
        nanosecond = nanosecond.saturating_add(1_000_000_000);
    }
    chrono::DateTime::<chrono::Utc>::from_timestamp(second, nanosecond as u32)
        .unwrap_or_else(chrono::Utc::now)
}

/// Extract a short preview string from a Message-kind tape entry's
/// payload. Returns `None` when no usable text is found (e.g. multimodal
/// payloads with no text segment).
fn extract_message_preview_text(payload: &Value) -> Option<String> {
    const MAX_PREVIEW_CHARS: usize = 200;

    let raw = payload
        .get("content")
        .and_then(|c| match c {
            Value::String(s) => Some(s.clone()),
            Value::Array(arr) => {
                // Multimodal: pick the first text segment.
                arr.iter()
                    .find_map(|seg| seg.get("text").and_then(Value::as_str).map(str::to_owned))
            }
            _ => None,
        })
        .or_else(|| {
            payload
                .get("text")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })?;

    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(MAX_PREVIEW_CHARS).collect())
}

fn score_search_candidate(
    normalized_query: &str,
    query_terms: &[String],
    searchable_text: &str,
    query_scorer: Option<&RatioBatchComparator<char>>,
) -> Option<f64> {
    if searchable_text.is_empty() {
        return None;
    }

    let exact_match = searchable_text.contains(normalized_query);
    let matched_terms = query_terms
        .iter()
        .filter(|term| searchable_text.contains(term.as_str()))
        .count();
    let term_coverage = if query_terms.is_empty() {
        0.0
    } else {
        matched_terms as f64 / query_terms.len() as f64
    };
    let fuzzy_score = query_scorer.map_or(0.0, |scorer| scorer.similarity(searchable_text.chars()));

    let has_full_term_match = !query_terms.is_empty() && matched_terms == query_terms.len();
    let has_partial_term_match = query_terms.len() >= MIN_QUERY_TERM_MATCHES
        && matched_terms >= MIN_QUERY_TERM_MATCHES
        && term_coverage >= MIN_QUERY_TERM_COVERAGE;
    let has_fuzzy_match = fuzzy_score >= MIN_FUZZY_SCORE;

    if !(exact_match || has_full_term_match || has_partial_term_match || has_fuzzy_match) {
        return None;
    }

    let mut score = (term_coverage * 0.7) + (fuzzy_score * 0.3);
    if exact_match {
        score += EXACT_MATCH_BONUS;
    }
    if has_full_term_match {
        score += 0.35;
    }

    Some(score)
}

fn extract_query_terms(normalized_query: &str) -> Vec<String> {
    normalized_query
        .split_whitespace()
        .filter(|term| !term.is_empty())
        .map(str::to_owned)
        .collect()
}

fn normalize_search_text(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    let mut previous_was_space = true;

    for ch in text.nfkc().flat_map(char::to_lowercase) {
        if ch.is_whitespace() {
            if !previous_was_space {
                normalized.push(' ');
            }
            previous_was_space = true;
            continue;
        }

        normalized.push(ch);
        previous_was_space = false;
    }

    if normalized.ends_with(' ') {
        normalized.pop();
    }

    normalized
}

/// Extract searchable text from a message payload and metadata.
pub(super) fn extract_searchable_text(payload: &Value, metadata: Option<&Value>) -> String {
    let mut parts = Vec::new();
    if let Some(text) = payload.get("content").and_then(Value::as_str) {
        parts.push(text.to_owned());
    }

    let payload_json = serde_json::to_string(payload).unwrap_or_default();
    if !payload_json.is_empty() {
        parts.push(payload_json);
    }

    if let Some(metadata) = metadata {
        let metadata_json = serde_json::to_string(metadata).unwrap_or_default();
        if !metadata_json.is_empty() {
            parts.push(metadata_json);
        }
    }

    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use std::{path::Path, sync::Arc};

    use super::*;
    use crate::session::test_utils::{InMemorySessionIndex, create_test_session};

    /// Create a [`TapeService`] backed by a temporary directory.
    async fn temp_tape_service(dir: &Path) -> TapeService {
        let store = super::super::FileTapeStore::new(dir, dir).await.unwrap();
        TapeService::new(store)
    }

    #[tokio::test]
    async fn append_message_publishes_tape_appended() {
        use crate::notification::{
            BroadcastNotificationBus, KernelNotification, NotificationFilter,
        };

        let tmp = tempfile::tempdir().unwrap();
        let bus: NotificationBusRef = Arc::new(BroadcastNotificationBus::default());
        let svc = temp_tape_service(tmp.path())
            .await
            .with_notifications(bus.clone());

        let key = SessionKey::new();
        let key_raw = key.to_string();
        let mut rx = bus.subscribe(NotificationFilter::default()).await;

        svc.append_message(
            &key_raw,
            json!({"role": "assistant", "content": "hi"}),
            None,
        )
        .await
        .unwrap();

        // The bus capacity is 256; one publish should arrive without lag.
        let event = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .expect("publish timed out")
            .expect("publish failed");
        match event {
            KernelNotification::TapeAppended {
                session_key, role, ..
            } => {
                assert_eq!(session_key, key);
                assert_eq!(role.as_deref(), Some("assistant"));
            }
            other => panic!("unexpected notification: {other:?}"),
        }
    }

    #[tokio::test]
    async fn build_anchor_tree_single_session() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = temp_tape_service(tmp.path()).await;
        let sessions = Arc::new(InMemorySessionIndex::new());

        let key = SessionKey::new();
        let key_raw = key.to_string();
        create_test_session(&sessions, &key, None).await;
        svc.ensure_bootstrap_anchor(&key_raw).await.unwrap();
        svc.handoff(
            &key_raw,
            "topic/first",
            HandoffState {
                summary: Some("first topic".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let tree = svc.build_anchor_tree(&key_raw, &*sessions).await.unwrap();
        assert_eq!(tree.root.session_key, key_raw);
        assert_eq!(tree.current_session, tree.root.session_key);
        assert_eq!(tree.root.anchors.len(), 2);
    }

    #[tokio::test]
    async fn build_anchor_tree_with_forks() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = temp_tape_service(tmp.path()).await;
        let sessions = Arc::new(InMemorySessionIndex::new());

        let root = SessionKey::new();
        let root_raw = root.to_string();
        create_test_session(&sessions, &root, None).await;
        svc.ensure_bootstrap_anchor(&root_raw).await.unwrap();
        svc.handoff(&root_raw, "topic/a", HandoffState::default())
            .await
            .unwrap();

        let fork = SessionKey::new();
        let fork_raw = fork.to_string();
        let mut metadata = None;
        super::super::set_fork_metadata(&mut metadata, &root_raw, "topic/a");
        create_test_session(&sessions, &fork, metadata).await;
        svc.ensure_bootstrap_anchor(&fork_raw).await.unwrap();

        let tree = svc.build_anchor_tree(&fork_raw, &*sessions).await.unwrap();
        assert_eq!(tree.root.session_key, root_raw);
        assert_eq!(tree.current_session, fork_raw);
        assert_eq!(tree.root.forks.len(), 1);
        assert_eq!(tree.root.forks[0].at_anchor, "topic/a");
        assert_eq!(tree.root.forks[0].branch.session_key, tree.current_session);
    }

    #[tokio::test]
    async fn build_llm_context_with_user_inserts_after_system_prompt() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = temp_tape_service(tmp.path()).await;
        let tape = "test-session";

        // Bootstrap anchor + system entry + user message.
        svc.ensure_bootstrap_anchor(tape).await.unwrap();
        svc.append_message(
            tape,
            json!({"role": "system", "content": "You are a helpful assistant."}),
            None,
        )
        .await
        .unwrap();
        svc.append_message(tape, json!({"role": "user", "content": "hi"}), None)
            .await
            .unwrap();

        // Write a user note.
        svc.append_user_note("alice", "fact", "likes Rust")
            .await
            .unwrap();

        let messages = svc
            .build_llm_context_with_user(tape, "alice")
            .await
            .unwrap();

        // The first message should still be the system prompt, not the user
        // memory note.
        assert_eq!(messages[0].role, crate::llm::Role::System);
        let first_text = match &messages[0].content {
            crate::llm::MessageContent::Text(t) => t.as_str(),
            _ => panic!("expected text"),
        };
        assert!(
            first_text.contains("helpful assistant"),
            "first message should be the original system prompt"
        );

        // The user memory message should be the second message (after the system
        // prompt but before conversation messages).
        assert_eq!(messages[1].role, crate::llm::Role::System);
        let second_text = match &messages[1].content {
            crate::llm::MessageContent::Text(t) => t.as_str(),
            _ => panic!("expected text"),
        };
        assert!(
            second_text.contains("[User Memory]"),
            "second message should be user memory"
        );
        assert!(second_text.contains("likes Rust"));

        // The conversation user message should follow.
        assert_eq!(messages[2].role, crate::llm::Role::User);
    }

    #[tokio::test]
    async fn build_llm_context_with_user_no_notes_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = temp_tape_service(tmp.path()).await;
        let tape = "test-session-2";

        svc.ensure_bootstrap_anchor(tape).await.unwrap();
        svc.append_message(tape, json!({"role": "user", "content": "hello"}), None)
            .await
            .unwrap();

        let messages = svc
            .build_llm_context_with_user(tape, "nobody")
            .await
            .unwrap();

        // No user notes → no injected system message, just the user message.
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, crate::llm::Role::User);
    }

    #[tokio::test]
    async fn handoff_hides_old_messages_from_default_context_but_search_can_recall_them() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = temp_tape_service(tmp.path()).await;
        let tape = "handoff-search";

        svc.ensure_bootstrap_anchor(tape).await.unwrap();
        svc.append_message(
            tape,
            json!({"role": "user", "content": "old fact: launch code banana-42"}),
            None,
        )
        .await
        .unwrap();
        svc.append_message(
            tape,
            json!({"role": "assistant", "content": "noted the old fact"}),
            None,
        )
        .await
        .unwrap();

        svc.handoff(
            tape,
            "topic/rotated",
            HandoffState {
                summary: Some("We recorded an old fact before rotating context.".into()),
                next_steps: Some("Search the tape if you need the hidden fact again.".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        svc.append_message(
            tape,
            json!({"role": "user", "content": "new topic only"}),
            None,
        )
        .await
        .unwrap();

        let context = svc.build_llm_context(tape).await.unwrap();
        let rendered_context = format!("{context:?}");
        assert!(
            !rendered_context.contains("banana-42"),
            "old message should be outside the default post-handoff context"
        );
        assert!(
            rendered_context.contains("new topic only"),
            "new message should remain in the default context"
        );

        let search_hits = svc.search(tape, "banana-42", 10, false).await.unwrap();
        assert_eq!(search_hits.len(), 1);
        assert_eq!(
            search_hits[0]
                .payload
                .get("content")
                .and_then(Value::as_str)
                .unwrap(),
            "old fact: launch code banana-42"
        );
    }

    #[tokio::test]
    async fn search_matches_high_overlap_multiterm_chinese_queries() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = temp_tape_service(tmp.path()).await;
        let tape = "search-chinese-overlap";

        svc.ensure_bootstrap_anchor(tape).await.unwrap();
        svc.append_message(
            tape,
            json!({"role": "user", "content": "我看了下飞日本的机票价格 福冈要2748元，好贵"}),
            None,
        )
        .await
        .unwrap();
        svc.append_message(
            tape,
            json!({"role": "user", "content": "上海今天下雨，晚点再看别的安排"}),
            None,
        )
        .await
        .unwrap();

        let hits = svc
            .search(tape, "上海 福冈 机票 价格", 10, false)
            .await
            .unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0]
                .payload
                .get("content")
                .and_then(Value::as_str)
                .unwrap(),
            "我看了下飞日本的机票价格 福冈要2748元，好贵"
        );
    }

    #[tokio::test]
    async fn search_includes_message_metadata_text() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = temp_tape_service(tmp.path()).await;
        let tape = "search-message-metadata";

        svc.ensure_bootstrap_anchor(tape).await.unwrap();
        svc.append_message(
            tape,
            json!({"role": "assistant", "content": "记录好了"}),
            Some(json!({
                "tags": ["travel", "fare"],
                "note": "上海 福冈 机票 价格 2748"
            })),
        )
        .await
        .unwrap();

        let hits = svc
            .search(tape, "上海 福冈 机票 价格", 10, false)
            .await
            .unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0]
                .payload
                .get("content")
                .and_then(Value::as_str)
                .unwrap(),
            "记录好了"
        );
    }

    #[test]
    fn score_search_candidate_handles_query_longer_than_source_without_panicking() {
        let query = normalize_search_text("what is the hidden credential");
        let terms = extract_query_terms(&query);
        let scorer = RatioBatchComparator::new(query.chars());

        let score = score_search_candidate(&query, &terms, "credential", Some(&scorer));

        assert!(score.is_none());
    }

    #[tokio::test]
    async fn tape_info_estimated_context_tokens() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = temp_tape_service(tmp.path()).await;
        let tape = "test-estimated-tokens";

        // User message (no usage)
        svc.append_message(tape, json!({"role": "user", "content": "hello"}), None)
            .await
            .unwrap();

        // Assistant message with usage metadata
        svc.append_message(
            tape,
            json!({"role": "assistant", "content": "hi there, how can I help?"}),
            Some(json!({"usage": {"prompt_tokens": 500, "completion_tokens": 100, "total_tokens": 600}})),
        )
        .await
        .unwrap();

        // Another user message after (no usage)
        svc.append_message(
            tape,
            json!({"role": "user", "content": "tell me about rust"}),
            None,
        )
        .await
        .unwrap();

        let info = svc.info(tape).await.unwrap();

        // 500 prompt + 100 completion = 600, plus ~chars/4 for the last user message
        assert!(info.estimated_context_tokens >= 600);
        assert!(info.estimated_context_tokens < 700); // chars/4 adds a small amount
    }

    #[tokio::test]
    async fn tape_info_estimated_context_tokens_no_usage() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = temp_tape_service(tmp.path()).await;
        let tape = "test-estimated-no-usage";

        // Only user messages (no usage metadata anywhere)
        svc.append_message(
            tape,
            json!({"role": "user", "content": "hello world"}),
            None,
        )
        .await
        .unwrap();

        let info = svc.info(tape).await.unwrap();

        // No usage data, so all estimation via chars/4
        assert!(info.estimated_context_tokens > 0);
    }

    #[tokio::test]
    async fn checkout_anchor_creates_forked_tape() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = temp_tape_service(tmp.path()).await;

        let source = "test-checkout-source";
        svc.ensure_bootstrap_anchor(source).await.unwrap();
        svc.handoff(
            source,
            "topic/a",
            HandoffState {
                summary: Some("discussed A".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        svc.append_message(
            source,
            serde_json::json!({"role":"user","content":"after anchor"}),
            None,
        )
        .await
        .unwrap();

        let target = "test-checkout-target";
        svc.checkout_anchor(source, "topic/a", target)
            .await
            .unwrap();

        let entries = svc.entries(target).await.unwrap();
        // Should have entries up to and including the "topic/a" anchor
        assert!(entries.iter().any(|e| {
            e.kind == TapEntryKind::Anchor
                && e.payload.get("name").and_then(|v| v.as_str()) == Some("topic/a")
        }));
        // Should NOT have the "after anchor" message
        assert!(!entries.iter().any(|e| {
            e.payload.get("content").and_then(|v| v.as_str()) == Some("after anchor")
        }));
    }

    #[tokio::test]
    async fn tape_index_matches_linear_scan() {
        // Build a small tape with mixed kinds and several anchors, then assert
        // the store's index-backed query methods return the same results as a
        // hand-written linear scan over the full entry list.  This guards the
        // index against drifting from `read_entries`.
        let tmp = tempfile::tempdir().unwrap();
        let svc = temp_tape_service(tmp.path()).await;
        let tape = "index-roundtrip";

        svc.ensure_bootstrap_anchor(tape).await.unwrap();
        svc.append_message(tape, json!({"role": "user", "content": "one"}), None)
            .await
            .unwrap();
        svc.handoff(tape, "topic/alpha", HandoffState::default())
            .await
            .unwrap();
        svc.append_message(tape, json!({"role": "user", "content": "two"}), None)
            .await
            .unwrap();
        svc.append_user_note("alice", "fact", "likes Rust")
            .await
            .unwrap();
        svc.handoff(tape, "topic/beta", HandoffState::default())
            .await
            .unwrap();
        svc.append_message(tape, json!({"role": "user", "content": "three"}), None)
            .await
            .unwrap();
        // A repeated anchor name to make sure we resolve to the most recent
        // occurrence rather than the first.
        svc.handoff(tape, "topic/alpha", HandoffState::default())
            .await
            .unwrap();
        svc.append_message(tape, json!({"role": "user", "content": "four"}), None)
            .await
            .unwrap();

        let entries = svc.entries(tape).await.unwrap();

        // Linear-scan reference values.
        let expected_last_anchor_id = entries
            .iter()
            .rev()
            .find(|e| e.kind == TapEntryKind::Anchor)
            .map(|e| e.id);
        let expected_alpha_id = entries
            .iter()
            .rev()
            .find(|e| {
                e.kind == TapEntryKind::Anchor
                    && e.payload.get("name").and_then(Value::as_str) == Some("topic/alpha")
            })
            .map(|e| e.id);
        let expected_beta_id = entries
            .iter()
            .rev()
            .find(|e| {
                e.kind == TapEntryKind::Anchor
                    && e.payload.get("name").and_then(Value::as_str) == Some("topic/beta")
            })
            .map(|e| e.id);
        let expected_anchor_count = entries
            .iter()
            .filter(|e| e.kind == TapEntryKind::Anchor)
            .count();
        let expected_message_ids: Vec<u64> = entries
            .iter()
            .filter(|e| e.kind == TapEntryKind::Message)
            .map(|e| e.id)
            .collect();

        // Index-backed values.
        let store = svc.store();
        let actual_last_anchor_id = store.last_anchor_id(tape).await.unwrap();
        let actual_alpha_id = store
            .last_anchor_id_by_name(tape, "topic/alpha")
            .await
            .unwrap();
        let actual_beta_id = store
            .last_anchor_id_by_name(tape, "topic/beta")
            .await
            .unwrap();
        let actual_missing = store
            .last_anchor_id_by_name(tape, "topic/nope")
            .await
            .unwrap();
        let actual_anchors = store
            .entries_by_kind(tape, TapEntryKind::Anchor)
            .await
            .unwrap();
        let actual_messages = store
            .entries_by_kind(tape, TapEntryKind::Message)
            .await
            .unwrap();

        assert_eq!(actual_last_anchor_id, expected_last_anchor_id);
        assert_eq!(actual_alpha_id, expected_alpha_id);
        assert_eq!(actual_beta_id, expected_beta_id);
        assert_eq!(actual_missing, None);
        assert_eq!(actual_anchors.len(), expected_anchor_count);
        assert!(
            actual_anchors
                .iter()
                .all(|e| e.kind == TapEntryKind::Anchor)
        );
        assert_eq!(
            actual_messages.iter().map(|e| e.id).collect::<Vec<_>>(),
            expected_message_ids
        );
    }

    #[tokio::test]
    async fn tape_index_survives_fork_merge() {
        // Forking clones the cache via `copy_to`; merging copies fork-local
        // entries back via `copy_from`.  Both paths must keep the index in
        // sync — verify by exercising fork_tape and querying the parent
        // afterwards.
        let tmp = tempfile::tempdir().unwrap();
        let svc = temp_tape_service(tmp.path()).await;
        let tape = "index-fork-merge";

        svc.ensure_bootstrap_anchor(tape).await.unwrap();
        svc.handoff(tape, "topic/before-fork", HandoffState::default())
            .await
            .unwrap();

        let svc_for_fork = svc.clone();
        svc.fork_tape(tape, None, |fork| async move {
            svc_for_fork
                .append_message(&fork, json!({"role": "user", "content": "in fork"}), None)
                .await?;
            svc_for_fork
                .handoff(&fork, "topic/in-fork", HandoffState::default())
                .await?;
            Ok(())
        })
        .await
        .unwrap();

        // After merge, the parent tape's index must know about the new anchor
        // that was created on the fork and copied back.
        let store = svc.store();
        let in_fork = store
            .last_anchor_id_by_name(tape, "topic/in-fork")
            .await
            .unwrap();
        assert!(
            in_fork.is_some(),
            "anchor created in fork should be visible via index after merge"
        );

        // The most recent anchor on the parent should now be the merged one.
        let last = store.last_anchor_id(tape).await.unwrap();
        assert_eq!(last, in_fork);
    }

    #[tokio::test]
    async fn rebuild_messages_merges_system_messages() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = temp_tape_service(tmp.path()).await;
        let tape = "test-merge-sys";

        // Seed the tape with a user message then an anchor with summary
        svc.append_message(
            tape,
            serde_json::json!({"role":"user","content":"hello"}),
            None,
        )
        .await
        .unwrap();
        svc.handoff(
            tape,
            "topic/done",
            super::HandoffState {
                summary: Some("previous context".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        // New conversation after anchor
        svc.append_message(
            tape,
            serde_json::json!({"role":"user","content":"hi again"}),
            None,
        )
        .await
        .unwrap();

        let messages = svc
            .rebuild_messages_for_llm(tape, None, "main system prompt")
            .await
            .unwrap();

        // There should be exactly ONE system message at the front (merged)
        let system_msgs: Vec<_> = messages
            .iter()
            .take_while(|m| m.role == crate::llm::Role::System)
            .collect();
        assert_eq!(
            system_msgs.len(),
            1,
            "expected exactly 1 leading system message, got {}",
            system_msgs.len()
        );

        let text = system_msgs[0].content.as_text();
        assert!(text.contains("main system prompt"), "missing main prompt");
        assert!(text.contains("previous context"), "missing anchor context");
    }

    // ---- FTS lifecycle integration tests ----

    /// Create a [`TapeService`] with FTS enabled via an in-memory SQLite pool.
    async fn temp_tape_service_with_fts(dir: &Path) -> TapeService {
        let pools = crate::testing::build_memory_diesel_pools().await;
        let store = super::super::FileTapeStore::new(dir, dir).await.unwrap();
        TapeService::with_fts(store, pools)
    }

    #[tokio::test]
    async fn fts_reset_clears_index() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = temp_tape_service_with_fts(tmp.path()).await;
        let tape = "fts-reset-test";

        // Append and index a message.
        svc.append_message(tape, json!({"content": "unique-token-xyz"}), None)
            .await
            .unwrap();

        // FTS search should find it (triggers backfill).
        let hits = svc
            .search(tape, "unique-token-xyz", 10, false)
            .await
            .unwrap();
        assert!(
            !hits.is_empty(),
            "should find the message via brute-force or FTS"
        );

        // Reset the tape.
        svc.reset(tape, false).await.unwrap();

        // After reset, FTS index should be cleared — search returns nothing.
        let hits = svc
            .search(tape, "unique-token-xyz", 10, false)
            .await
            .unwrap();
        assert!(hits.is_empty(), "FTS should be cleared after reset");
    }

    #[tokio::test]
    async fn fts_delete_tape_clears_index() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = temp_tape_service_with_fts(tmp.path()).await;
        let tape = "fts-delete-test";

        svc.append_message(tape, json!({"content": "delete-me-token"}), None)
            .await
            .unwrap();

        // Trigger backfill via search.
        let hits = svc
            .search(tape, "delete-me-token", 10, false)
            .await
            .unwrap();
        assert!(!hits.is_empty());

        // Delete the tape.
        svc.delete_tape(tape).await.unwrap();

        // FTS index should be empty.
        let fts = svc.fts.as_ref().expect("FTS should be Some");
        let hwm = fts.last_indexed_id(tape).await.unwrap();
        assert_eq!(hwm, 0, "HWM should be 0 after delete");
    }

    #[tokio::test]
    async fn search_across_tapes_empty_query() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = temp_tape_service_with_fts(tmp.path()).await;
        svc.append_message("tape-a", json!({"content": "hello"}), None)
            .await
            .unwrap();

        let hits = svc.search_across_tapes("", 10).await.unwrap();
        assert!(hits.is_empty());

        let hits = svc.search_across_tapes("   ", 10).await.unwrap();
        assert!(hits.is_empty());
    }

    #[tokio::test]
    async fn search_across_tapes_single_tape_match() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = temp_tape_service_with_fts(tmp.path()).await;

        svc.append_message("tape-a", json!({"content": "rustacean loves ferris"}), None)
            .await
            .unwrap();
        svc.append_message("tape-b", json!({"content": "unrelated python snake"}), None)
            .await
            .unwrap();

        let hits = svc.search_across_tapes("ferris", 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].tape_name, "tape-a");
        assert!(
            hits[0]
                .entry
                .payload
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .contains("ferris")
        );
    }

    #[tokio::test]
    async fn search_across_tapes_multi_tape_attribution() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = temp_tape_service_with_fts(tmp.path()).await;

        svc.append_message("tape-a", json!({"content": "alpha keyword tape-a"}), None)
            .await
            .unwrap();
        svc.append_message("tape-b", json!({"content": "beta keyword tape-b"}), None)
            .await
            .unwrap();
        svc.append_message("tape-c", json!({"content": "gamma keyword tape-c"}), None)
            .await
            .unwrap();

        let hits = svc.search_across_tapes("keyword", 10).await.unwrap();
        assert_eq!(hits.len(), 3);
        let tapes: std::collections::HashSet<String> =
            hits.iter().map(|h| h.tape_name.clone()).collect();
        assert!(tapes.contains("tape-a"));
        assert!(tapes.contains("tape-b"));
        assert!(tapes.contains("tape-c"));

        // Attribution check: each hit's tape_name must match the tape
        // that actually holds the content.
        for hit in &hits {
            let content = hit
                .entry
                .payload
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            assert!(
                content.contains(&hit.tape_name),
                "hit content {content:?} should reference tape {:?}",
                hit.tape_name
            );
        }
    }

    #[tokio::test]
    async fn search_across_tapes_brute_force_fallback() {
        // No FTS — exercises the brute-force path for attribution.
        let tmp = tempfile::tempdir().unwrap();
        let svc = temp_tape_service(tmp.path()).await;

        svc.append_message("tape-x", json!({"content": "needle in x"}), None)
            .await
            .unwrap();
        svc.append_message("tape-y", json!({"content": "needle in y"}), None)
            .await
            .unwrap();

        let hits = svc.search_across_tapes("needle", 10).await.unwrap();
        assert_eq!(hits.len(), 2);
        for hit in &hits {
            let content = hit
                .entry
                .payload
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            assert!(content.contains(&hit.tape_name[hit.tape_name.len() - 1..]));
        }
    }
}
