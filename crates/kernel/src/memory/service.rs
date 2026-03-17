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

use std::future::Future;

use rapidfuzz::fuzz::RatioBatchComparator;
use serde_json::{Map, Value, json};
use unicode_normalization::UnicodeNormalization;

use super::{
    AnchorNode, AnchorSummary, AnchorTree, FileTapeStore, ForkEdge, HandoffState, SessionBranch,
    TapEntry, TapEntryKind, TapResult, get_fork_metadata,
};
use crate::session::{SessionError, SessionIndex, SessionKey};

thread_local! {
    /// Per-thread current tape context used while executing fork closures.
    static TAPE_CONTEXT: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
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
    score: f64,
    entry: TapEntry,
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

    /// Count the number of [`TapEntryKind::Message`] entries in a tape.
    pub async fn message_count(&self, tape_name: &str) -> TapResult<usize> {
        let entries = self.entries(tape_name).await?;
        Ok(entries
            .iter()
            .filter(|e| e.kind == TapEntryKind::Message)
            .count())
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

        // Compute estimated context tokens for entries since last anchor.
        let anchor_id = anchors.last().map(|a| a.id).unwrap_or(0);
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
            anchors: anchors.len(),
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
        let entries = self.entries(source).await?;

        let anchor_id = entries
            .iter()
            .rev()
            .find(|e| {
                e.kind == TapEntryKind::Anchor
                    && e.payload.get("name").and_then(|v| v.as_str()) == Some(anchor_name)
            })
            .map(|e| e.id)
            .ok_or_else(|| super::TapError::State {
                message: format!("anchor not found: {anchor_name}"),
            })?;

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

    /// Search message entries using ranked Unicode-aware text matching.
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
        let query_terms = extract_query_terms(&normalized_query);
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
                    &normalized_query,
                    &query_terms,
                    &searchable_text,
                    query_scorer.as_ref(),
                ) else {
                    continue;
                };
                results.push(SearchMatch { score, entry });
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
            .map_err(map_session_error)?;

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
                .map_err(map_session_error)?
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
        let entries = self.entries(session_key).await?;
        Ok(entries
            .into_iter()
            .filter(|entry| entry.kind == TapEntryKind::Anchor)
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

fn map_session_error(error: SessionError) -> super::TapError {
    // Keep a tape-local error surface for callers in memory subsystem.
    super::TapError::State {
        message: error.to_string(),
    }
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
fn extract_searchable_text(payload: &Value, metadata: Option<&Value>) -> String {
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
}
