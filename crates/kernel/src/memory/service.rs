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

use std::{future::Future, sync::OnceLock};

use regex::Regex;
use serde_json::{Map, Value, json};

use super::{AnchorSummary, FileTapeStore, HandoffState, TapEntry, TapEntryKind, TapResult};

thread_local! {
    /// Per-thread current tape context used while executing fork closures.
    static TAPE_CONTEXT: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Token-matching regex reused by the fuzzy search fallback.
static WORD_PATTERN: OnceLock<Regex> = OnceLock::new();
/// Queries shorter than this skip fuzzy matching to avoid noisy results.
const MIN_FUZZY_QUERY_LENGTH: usize = 3;
/// Minimum normalized similarity percentage for a fuzzy hit.
const MIN_FUZZY_SCORE: usize = 80;
/// Hard cap on fuzzy candidates checked per tape read.
const MAX_FUZZY_CANDIDATES: usize = 128;

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
}

/// Get the current tape in contextual execution, mirroring Bub's
/// `current_tape`.
///
/// This is mainly useful while executing [`TapeService::fork_tape`], where the
/// closure temporarily runs against a forked tape context.
pub fn current_tape() -> String {
    TAPE_CONTEXT.with(|current| current.borrow().clone().unwrap_or_else(|| "-".to_owned()))
}

/// Tape helper with app-specific operations.
///
/// Unlike the low-level [`FileTapeStore`], `TapeService` provides higher-level
/// workflows (anchors, fork/merge, search, LLM context building). It is **not**
/// bound to a specific tape — every method accepts a `tape_name` parameter so a
/// single instance can serve all sessions.
#[derive(Debug, Clone)]
pub struct TapeService {
    store: FileTapeStore,
}

impl TapeService {
    /// Create a service backed by the given store.
    pub fn new(store: FileTapeStore) -> Self { Self { store } }

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
        Ok(entries.into_iter().filter(|e| id_set.contains(&e.id)).collect())
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
            .handoff(tape_name, "session/start", HandoffState { owner: Some("human".into()), ..Default::default() })
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
        self.store
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
        self.from_last_anchor(tape_name, None).await
    }

    /// Append an event entry.
    pub async fn append_event(&self, tape_name: &str, name: &str, data: Value) -> TapResult<()> {
        self.store
            .append(
                tape_name,
                TapEntryKind::Event,
                json!({"name": name, "data": data}),
                None,
            )
            .await?;
        Ok(())
    }

    /// Append a system entry.
    pub async fn append_system(&self, tape_name: &str, content: &str) -> TapResult<()> {
        self.store
            .append(
                tape_name,
                TapEntryKind::System,
                json!({"content": content}),
                None,
            )
            .await?;
        Ok(())
    }

    /// Append a message entry.
    pub async fn append_message(
        &self,
        tape_name: &str,
        payload: Value,
        metadata: Option<Value>,
    ) -> TapResult<TapEntry> {
        self.store
            .append(tape_name, TapEntryKind::Message, payload, metadata)
            .await
    }

    /// Append a tool-call entry.
    pub async fn append_tool_call(
        &self,
        tape_name: &str,
        payload: Value,
        metadata: Option<Value>,
    ) -> TapResult<TapEntry> {
        self.store
            .append(tape_name, TapEntryKind::ToolCall, payload, metadata)
            .await
    }

    /// Append a tool-result entry.
    pub async fn append_tool_result(
        &self,
        tape_name: &str,
        payload: Value,
        metadata: Option<Value>,
    ) -> TapResult<TapEntry> {
        self.store
            .append(tape_name, TapEntryKind::ToolResult, payload, metadata)
            .await
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

        // Load user tape and inject user context after any leading system
        // messages, so it appears between the system prompt and conversation
        // history.  This ensures the LLM's system prompt is never displaced.
        let user_tape = super::user_tape_name(user_id);
        let user_entries = self.entries(&user_tape).await?;
        if let Some(user_msg) = super::context::user_tape_context(&user_entries) {
            let insert_pos = messages
                .iter()
                .position(|m| m.role != crate::llm::Role::System)
                .unwrap_or(messages.len());
            messages.insert(insert_pos, user_msg);
        }

        Ok(messages)
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
        self.store
            .append(
                &user_tape,
                TapEntryKind::Note,
                serde_json::json!({
                    "category": category,
                    "content": content,
                }),
                None,
            )
            .await
    }

    /// Read all note entries from a user tape.
    pub async fn read_user_notes(&self, user_id: &str) -> TapResult<Vec<TapEntry>> {
        let user_tape = super::user_tape_name(user_id);
        let entries = self.entries(&user_tape).await?;
        Ok(entries
            .into_iter()
            .filter(|e| e.kind == TapEntryKind::Note)
            .collect())
    }

    /// Inspect current tape state without mutating it.
    pub async fn info(&self, tape_name: &str) -> TapResult<TapeInfo> {
        let entries = self.entries(tape_name).await?;
        let anchors = entries
            .iter()
            .filter(|entry| entry.kind == TapEntryKind::Anchor)
            .collect::<Vec<_>>();
        let last_anchor = anchors
            .last()
            .and_then(|entry| entry.payload.get("name"))
            .and_then(Value::as_str)
            .map(str::to_owned);

        let entries_since_last_anchor = if let Some(last) = anchors.last() {
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

        Ok(TapeInfo {
            name: tape_name.to_owned(),
            entries: entries.len(),
            anchors: anchors.len(),
            last_anchor,
            entries_since_last_anchor,
            last_token_usage,
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
        let handoff_state = HandoffState {
            owner: Some("human".into()),
            extra: archive_path.as_ref().map(|path| {
                json!({ "archived": path.to_string_lossy() })
            }),
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

    /// Return the most recent `limit` anchors, oldest-to-newest within the
    /// returned window.
    pub async fn anchors(&self, tape_name: &str, limit: usize) -> TapResult<Vec<AnchorSummary>> {
        let entries = self.entries(tape_name).await?;
        let anchor_entries: Vec<_> = entries
            .iter()
            .filter(|entry| entry.kind == TapEntryKind::Anchor)
            .collect();
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
        let entries = self.entries(tape_name).await?;
        let start_id = anchor_id(&entries, start);
        let end_id = anchor_id(&entries, end);
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
        let entries = self.entries(tape_name).await?;
        let anchor_id = anchor_id(&entries, anchor);
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
        let entries = self.entries(tape_name).await?;
        let last_anchor_id = entries
            .iter()
            .rev()
            .find(|entry| entry.kind == TapEntryKind::Anchor)
            .map(|entry| entry.id);
        Ok(entries
            .into_iter()
            .filter(|entry| last_anchor_id.is_none_or(|id| entry.id >= id))
            .filter(|entry| kind_matches(entry, kinds))
            .collect())
    }

    /// Search message entries using exact substring matching plus a lightweight
    /// fuzzy fallback.
    pub async fn search(
        &self,
        tape_name: &str,
        query: &str,
        limit: usize,
        all_tapes: bool,
    ) -> TapResult<Vec<TapEntry>> {
        let normalized_query = query.trim().to_lowercase();
        if normalized_query.is_empty() {
            return Ok(Vec::new());
        }

        let tape_names = if all_tapes {
            self.store.list_tapes().await?
        } else {
            vec![tape_name.to_owned()]
        };

        let mut results = Vec::new();
        for name in tape_names {
            let mut count = 0usize;
            let entries = self.store.read(&name).await?.unwrap_or_default();
            for entry in entries.into_iter().rev() {
                if entry.kind != TapEntryKind::Message {
                    continue;
                }
                let payload_text = extract_searchable_text(&entry.payload);
                if payload_text.to_lowercase().contains(&normalized_query)
                    || is_fuzzy_match(&normalized_query, &payload_text)
                {
                    results.push(entry);
                    count += 1;
                    if count >= limit {
                        break;
                    }
                }
            }
        }

        Ok(results)
    }

    /// Compact a tape by writing a compaction anchor that shrinks the default
    /// read set (`from_last_anchor`) without deleting any history.
    ///
    /// Old `Message`, `ToolCall`, and `ToolResult` entries remain on disk but
    /// fall outside the new anchor window and are therefore excluded from the
    /// default context view.  This preserves the append-only invariant while
    /// keeping the working set small.
    ///
    /// Returns the number of conversational entries moved out of the default
    /// view, or 0 if the tape was below the compaction threshold.
    pub async fn compact_tape(&self, tape_name: &str, keep_recent: usize) -> TapResult<usize> {
        let entries = self.entries(tape_name).await?;
        let total = entries.len();

        // Nothing to compact if the tape is small enough.
        if total <= keep_recent {
            return Ok(0);
        }

        // Count entries since the last anchor — if already within budget, skip.
        let last_anchor_pos = entries
            .iter()
            .rposition(|e| e.kind == TapEntryKind::Anchor);
        let entries_since_anchor = match last_anchor_pos {
            Some(pos) => total - pos - 1,
            None => total,
        };
        if entries_since_anchor <= keep_recent {
            return Ok(0);
        }

        // Determine how many conversational entries fall outside the kept window.
        let split_point = total.saturating_sub(keep_recent);
        let old_entries = &entries[..split_point];
        let conversational_kinds = [
            TapEntryKind::Message,
            TapEntryKind::ToolCall,
            TapEntryKind::ToolResult,
        ];
        let source_ids: Vec<u64> = old_entries
            .iter()
            .filter(|e| conversational_kinds.contains(&e.kind))
            .map(|e| e.id)
            .collect();
        let discarded = source_ids.len();

        if discarded == 0 {
            return Ok(0);
        }

        // Write a compaction anchor — from_last_anchor() will naturally narrow
        // the default read set to entries after this point.
        let handoff_state = HandoffState {
            summary: Some(format!(
                "Compacted: {discarded} old message/tool entries moved out of default view. \
                 Total history: {total} entries preserved on tape."
            )),
            owner: Some("system".into()),
            source_ids,
            extra: Some(json!({
                "compaction": {
                    "discarded_from_view": discarded,
                    "original_total": total,
                    "kept_recent": keep_recent,
                }
            })),
            ..Default::default()
        };

        self.handoff(tape_name, "compaction", handoff_state).await?;
        Ok(discarded)
    }

    /// List all tape names known to the underlying store.
    pub async fn list_tapes(&self) -> TapResult<Vec<String>> { self.store.list_tapes().await }
}

/// Find the most recent anchor ID for a named anchor.
fn anchor_id(entries: &[TapEntry], name: &str) -> Option<u64> {
    entries
        .iter()
        .rev()
        .find(|entry| {
            entry.kind == TapEntryKind::Anchor
                && entry.payload.get("name").and_then(Value::as_str) == Some(name)
        })
        .map(|entry| entry.id)
}

/// Apply an optional kind filter to one entry.
fn kind_matches(entry: &TapEntry, kinds: Option<&[TapEntryKind]>) -> bool {
    kinds.is_none_or(|kinds| kinds.iter().any(|kind| kind == &entry.kind))
}

/// Check whether a message payload approximately matches the normalized query.
fn is_fuzzy_match(normalized_query: &str, payload_text: &str) -> bool {
    if normalized_query.len() < MIN_FUZZY_QUERY_LENGTH {
        return false;
    }

    let word_pattern = WORD_PATTERN.get_or_init(|| Regex::new(r"[a-z0-9_/-]+").expect("regex"));
    let query_tokens = word_pattern
        .find_iter(normalized_query)
        .map(|m| m.as_str().to_owned())
        .collect::<Vec<_>>();
    if query_tokens.is_empty() {
        return false;
    }
    let query_phrase = query_tokens.join(" ");
    let window_size = query_tokens.len();

    let source_tokens = word_pattern
        .find_iter(&payload_text.to_lowercase())
        .map(|m| m.as_str().to_owned())
        .collect::<Vec<_>>();
    if source_tokens.is_empty() {
        return false;
    }

    let mut candidates = Vec::new();
    for token in &source_tokens {
        candidates.push(token.clone());
        if candidates.len() >= MAX_FUZZY_CANDIDATES {
            break;
        }
    }

    if window_size > 1 {
        for idx in 0..source_tokens
            .len()
            .saturating_sub(window_size)
            .saturating_add(1)
        {
            candidates.push(source_tokens[idx..idx + window_size].join(" "));
            if candidates.len() >= MAX_FUZZY_CANDIDATES {
                break;
            }
        }
    }

    candidates
        .iter()
        .any(|candidate| similarity_percent(&query_phrase, candidate) >= MIN_FUZZY_SCORE)
}

/// Convert Levenshtein distance into a 0-100 similarity score.
fn similarity_percent(a: &str, b: &str) -> usize {
    let distance = levenshtein(a, b);
    let max_len = a.chars().count().max(b.chars().count());
    if max_len == 0 {
        return 100;
    }
    (((max_len.saturating_sub(distance)) * 100) / max_len).min(100)
}

/// Compute character-level edit distance for the fuzzy search fallback.
fn levenshtein(a: &str, b: &str) -> usize {
    let b_chars = b.chars().collect::<Vec<_>>();
    let mut costs = (0..=b_chars.len()).collect::<Vec<_>>();

    for (i, a_char) in a.chars().enumerate() {
        let mut last = i;
        costs[0] = i + 1;
        for (j, b_char) in b_chars.iter().enumerate() {
            let current = costs[j + 1];
            let substitution = if a_char == *b_char { last } else { last + 1 };
            let insertion = current + 1;
            let deletion = costs[j] + 1;
            costs[j + 1] = substitution.min(insertion).min(deletion);
            last = current;
        }
    }

    costs[b_chars.len()]
}

/// Extract searchable text from a message payload, preferring the `content`
/// string field to avoid a full JSON serialization round-trip.
fn extract_searchable_text(payload: &Value) -> String {
    if let Some(text) = payload.get("content").and_then(Value::as_str) {
        return text.to_owned();
    }
    serde_json::to_string(payload).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    /// Create a [`TapeService`] backed by a temporary directory.
    async fn temp_tape_service(dir: &Path) -> TapeService {
        let store = super::super::FileTapeStore::new(dir, dir).await.unwrap();
        TapeService::new(store)
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
    async fn compact_tape_below_threshold_is_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = temp_tape_service(tmp.path()).await;
        let tape = "compact-noop";

        svc.ensure_bootstrap_anchor(tape).await.unwrap();
        for i in 0..5 {
            svc.append_message(
                tape,
                json!({"role": "user", "content": format!("msg {i}")}),
                None,
            )
            .await
            .unwrap();
        }

        let discarded = svc.compact_tape(tape, 100).await.unwrap();
        assert_eq!(discarded, 0);

        // Entries should be unchanged.
        let entries = svc.entries(tape).await.unwrap();
        assert_eq!(entries.len(), 6); // 1 anchor + 5 messages
    }

    #[tokio::test]
    async fn compact_tape_discards_old_messages_preserves_notes() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = temp_tape_service(tmp.path()).await;
        let tape = "compact-test";

        // Create: 1 anchor + 1 note + 10 messages = 12 entries total.
        svc.ensure_bootstrap_anchor(tape).await.unwrap();
        svc.append_user_note("alice", "fact", "likes Rust")
            .await
            .unwrap();
        // Append messages to the same tape (not user tape) for testing.
        for i in 0..10 {
            svc.append_message(
                tape,
                json!({"role": "user", "content": format!("msg {i}")}),
                None,
            )
            .await
            .unwrap();
        }

        // Note was written to user tape, so add a Note directly to this tape for
        // testing.
        svc.store()
            .append(
                tape,
                TapEntryKind::Note,
                json!({"category": "fact", "content": "test note"}),
                None,
            )
            .await
            .unwrap();

        // Now: 1 anchor + 10 messages + 1 note = 12 entries.
        let before = svc.entries(tape).await.unwrap();
        assert_eq!(before.len(), 12);

        // Compact keeping only the 3 most recent entries.
        let discarded = svc.compact_tape(tape, 3).await.unwrap();

        // Old section had 9 entries (1 anchor + 8 messages). Anchor is preserved,
        // 8 messages discarded.
        assert_eq!(discarded, 8);

        let after = svc.entries(tape).await.unwrap();
        // 1 summary + 1 anchor (preserved) + 3 recent = 5
        assert_eq!(after.len(), 5);
        assert_eq!(after[0].kind, TapEntryKind::Summary);
        assert_eq!(after[1].kind, TapEntryKind::Anchor);
    }

    #[tokio::test]
    async fn compact_tape_no_discardable_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = temp_tape_service(tmp.path()).await;
        let tape = "compact-only-anchors";

        // Create a tape with only anchors (non-discardable).
        for i in 0..5 {
            svc.handoff(tape, &format!("anchor-{i}"), HandoffState::default())
                .await
                .unwrap();
        }

        let entries = svc.entries(tape).await.unwrap();
        let total = entries.len();

        // Keep 2 recent, old section is all anchors → 0 discarded.
        let discarded = svc.compact_tape(tape, 2).await.unwrap();
        assert_eq!(discarded, 0);

        // Entries should be unchanged.
        let after = svc.entries(tape).await.unwrap();
        assert_eq!(after.len(), total);
    }
}
