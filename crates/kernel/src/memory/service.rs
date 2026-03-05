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

use super::{AnchorSummary, FileTapeStore, TapEntry, TapEntryKind, TapResult};

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

    /// Execute `func` against a forked tape. On success, merge the fork back
    /// into the parent tape. On failure, discard the fork so failed turns do
    /// not pollute the main tape.
    pub async fn fork_tape<T, F, Fut>(&self, tape_name: &str, func: F) -> TapResult<T>
    where
        F: FnOnce(String) -> Fut,
        Fut: Future<Output = TapResult<T>>,
    {
        let fork_name = self.store.fork(tape_name).await?;

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
            .handoff(tape_name, "session/start", Some(json!({"owner": "human"})))
            .await?;
        Ok(())
    }

    /// Append an anchor and return entries from the most recent anchor onward.
    pub async fn handoff(
        &self,
        tape_name: &str,
        name: &str,
        state: Option<Value>,
    ) -> TapResult<Vec<TapEntry>> {
        self.store
            .append(
                tape_name,
                TapEntryKind::Anchor,
                json!({
                    "name": name,
                    "state": state.unwrap_or(Value::Object(Map::new())),
                }),
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
            )
            .await?;
        Ok(())
    }

    /// Append a system entry.
    pub async fn append_system(&self, tape_name: &str, content: &str) -> TapResult<()> {
        self.store
            .append(tape_name, TapEntryKind::System, json!({"content": content}))
            .await?;
        Ok(())
    }

    /// Append a message entry.
    pub async fn append_message(&self, tape_name: &str, payload: Value) -> TapResult<TapEntry> {
        self.store
            .append(tape_name, TapEntryKind::Message, payload)
            .await
    }

    /// Append a tool-call entry.
    pub async fn append_tool_call(&self, tape_name: &str, payload: Value) -> TapResult<TapEntry> {
        self.store
            .append(tape_name, TapEntryKind::ToolCall, payload)
            .await
    }

    /// Append a tool-result entry.
    pub async fn append_tool_result(&self, tape_name: &str, payload: Value) -> TapResult<TapEntry> {
        self.store
            .append(tape_name, TapEntryKind::ToolResult, payload)
            .await
    }

    /// Build LLM-ready messages from tape entries since the last anchor.
    pub async fn build_llm_context(&self, tape_name: &str) -> TapResult<Vec<crate::llm::Message>> {
        let entries = self
            .from_last_anchor(
                tape_name,
                Some(&[
                    TapEntryKind::Message,
                    TapEntryKind::ToolCall,
                    TapEntryKind::ToolResult,
                ]),
            )
            .await?;
        super::context::default_tape_context(&entries)
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
            if entry.kind != TapEntryKind::Event
                || entry.payload.get("name") != Some(&Value::String("run".to_owned()))
            {
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
        let mut state = Map::new();
        state.insert("owner".to_owned(), Value::String("human".to_owned()));
        if let Some(path) = archive_path.as_ref() {
            state.insert(
                "archived".to_owned(),
                Value::String(path.to_string_lossy().into_owned()),
            );
        }
        let _ = self
            .handoff(tape_name, "session/start", Some(Value::Object(state)))
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
