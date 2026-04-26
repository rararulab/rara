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

//! Chat domain service — session/message/model management.
//!
//! [`SessionService`] is the primary entry point for all chat CRUD operations.
//! It holds references to the session index and tape store, and exposes
//! high-level methods for session management, model catalog queries, and
//! channel bindings.
//!
//! Session metadata is managed by [`SessionIndexRef`]. Message persistence has
//! moved to the tape subsystem via [`TapeService`].

use std::sync::Arc;

use chrono::Utc;
use rara_domain_shared::settings::{SettingsProvider, keys};
use rara_kernel::{
    cascade::{
        CascadeTrace, build_cascade, find_turn_boundaries, load_persisted_cascade, turn_slice,
    },
    channel::types::{
        ChannelType, ChatMessage, MessageContent, MessageRole, ToolCall as ChannelToolCall,
    },
    llm::{Message, Role},
    memory::{TapEntry, TapEntryKind, TapeSearchHit, TapeService},
    session::SessionIndexRef,
    trace::{ExecutionTrace, TraceService},
};
use rara_sessions::types::{ChannelBinding, SessionEntry, SessionKey, ThinkingLevel};
use serde_json::Value;
use tracing::{info, instrument};

use crate::chat::{
    error::ChatError,
    model_catalog::{ChatModel, ModelCatalog},
};

/// Sanitised view of an `llm.providers.<id>.*` settings group.
///
/// Surfaces only the fields the chat UI needs to render a picker —
/// raw API keys are intentionally replaced with the boolean
/// `has_api_key` so secrets never leave the backend via this endpoint.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct ProviderInfo {
    /// Provider id, as used in `llm.providers.<id>.*` keys and in the
    /// kernel `DriverRegistry` (e.g. `openrouter`, `kimi`, `minimax`).
    pub id:            String,
    /// Non-empty `default_model` value from settings. Providers without
    /// one are omitted from the list.
    pub default_model: String,
    /// Base URL for OpenAI-compatible endpoints, if configured.
    pub base_url:      Option<String>,
    /// Whether `llm.providers.<id>.api_key` has a non-empty value.
    pub has_api_key:   bool,
    /// Whether `llm.providers.<id>.enabled` is the literal string `"true"`.
    pub enabled:       bool,
}

/// Walk the flat settings map and assemble sanitised provider entries.
///
/// Provider ids with no `default_model` are skipped. Enabled providers
/// sort first, then providers with an api key, then the rest by id.
///
/// The `enabled` flag is read strictly from the literal string `"true"`
/// to match how the Settings UI writes the value — any other shape
/// (including `"1"`, `"yes"`, `"True"`) is treated as disabled.
pub(crate) fn collect_providers(
    settings: &std::collections::HashMap<String, String>,
) -> Vec<ProviderInfo> {
    use std::collections::HashMap;
    let mut by_id: HashMap<&str, HashMap<&str, &str>> = HashMap::new();
    for (key, value) in settings {
        let rest = match key.strip_prefix("llm.providers.") {
            Some(r) => r,
            None => continue,
        };
        // `rest` looks like `<id>.<field>` — or `<id>.<sub>.<more>` for
        // nested fields we do not care about. Split on the FIRST dot so
        // `id` ends at the first segment. Reject empty ids to shield
        // against malformed keys like `llm.providers..default_model`
        // producing a ghost entry.
        let (id, field) = match rest.split_once('.') {
            Some(pair) => pair,
            None => continue,
        };
        if id.is_empty() {
            continue;
        }
        by_id.entry(id).or_default().insert(field, value.as_str());
    }

    let mut entries: Vec<ProviderInfo> = by_id
        .into_iter()
        .filter_map(|(id, fields)| {
            let default_model = fields.get("default_model").copied().unwrap_or("").trim();
            if default_model.is_empty() {
                return None;
            }
            let base_url = fields
                .get("base_url")
                .copied()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            let has_api_key = fields
                .get("api_key")
                .copied()
                .is_some_and(|v| !v.trim().is_empty());
            let enabled = fields.get("enabled").copied() == Some("true");
            Some(ProviderInfo {
                id: id.to_owned(),
                default_model: default_model.to_owned(),
                base_url,
                has_api_key,
                enabled,
            })
        })
        .collect();

    entries.sort_by(|a, b| {
        let score = |e: &ProviderInfo| (i32::from(e.enabled) * 2) + (i32::from(e.has_api_key));
        let diff = score(b).cmp(&score(a));
        if diff.is_eq() { a.id.cmp(&b.id) } else { diff }
    });
    entries
}

#[cfg(test)]
mod provider_tests {
    use std::collections::HashMap;

    use super::{ProviderInfo, collect_providers};

    fn settings(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn omits_api_key_values_from_serialized_output() {
        // The whole point of the endpoint: raw key material must never
        // appear in the response body, only its presence as a boolean.
        let s = settings(&[
            ("llm.providers.kimi.api_key", "sk-secret-value"),
            ("llm.providers.kimi.default_model", "kimi-k2.5"),
            ("llm.providers.kimi.enabled", "true"),
        ]);
        let out = collect_providers(&s);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "kimi");
        assert!(out[0].has_api_key);

        let json = serde_json::to_string(&out).expect("serialize");
        assert!(
            !json.contains("sk-secret-value"),
            "serialized output leaked api_key: {json}"
        );
        // The boolean `has_api_key` field legitimately contains the
        // substring — check the value side of the JSON by looking for
        // the literal key name followed by a colon.
        assert!(
            !json.contains("\"api_key\":"),
            "serialized output should not expose raw `api_key` field: {json}"
        );
    }

    #[test]
    fn skips_providers_without_default_model() {
        let s = settings(&[
            ("llm.providers.configured.api_key", "abc"),
            ("llm.providers.configured.default_model", "m1"),
            ("llm.providers.no_model.api_key", "abc"),
        ]);
        let out = collect_providers(&s);
        let ids: Vec<&str> = out.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, vec!["configured"]);
    }

    #[test]
    fn ignores_malformed_empty_id_keys() {
        // `llm.providers..default_model` should not materialise a row
        // with id="" — protects against typos / partial writes.
        let s = settings(&[
            ("llm.providers..default_model", "m1"),
            ("llm.providers.real.default_model", "m2"),
        ]);
        let out = collect_providers(&s);
        let ids: Vec<&str> = out.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, vec!["real"]);
    }

    #[test]
    fn enabled_requires_literal_true_string() {
        // Mirrors the guard on `enabled`: only lowercase "true" counts.
        let s = settings(&[
            ("llm.providers.a.default_model", "m"),
            ("llm.providers.a.enabled", "True"),
            ("llm.providers.b.default_model", "m"),
            ("llm.providers.b.enabled", "true"),
            ("llm.providers.c.default_model", "m"),
            ("llm.providers.c.enabled", "1"),
        ]);
        let out = collect_providers(&s);
        let flags: HashMap<_, _> = out.iter().map(|p| (p.id.as_str(), p.enabled)).collect();
        assert_eq!(flags.get("a").copied(), Some(false));
        assert_eq!(flags.get("b").copied(), Some(true));
        assert_eq!(flags.get("c").copied(), Some(false));
    }

    #[test]
    fn sorts_enabled_first_then_api_key_then_id() {
        let s = settings(&[
            ("llm.providers.zeta.default_model", "m"),
            ("llm.providers.zeta.enabled", "true"),
            ("llm.providers.alpha.default_model", "m"),
            ("llm.providers.alpha.api_key", "k"),
            ("llm.providers.mid.default_model", "m"),
        ]);
        let out = collect_providers(&s);
        let ids: Vec<&str> = out.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, vec!["zeta", "alpha", "mid"]);
    }

    #[test]
    fn ignores_nested_subfields() {
        // `llm.providers.X.fallback.models` is a known rara key shape
        // that is NOT one of the fields we surface — it should be
        // quietly ignored rather than crash or alter the output.
        let s = settings(&[
            ("llm.providers.x.default_model", "m"),
            ("llm.providers.x.fallback.models", "a,b,c"),
        ]);
        let out: Vec<ProviderInfo> = collect_providers(&s);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "x");
        assert!(!out[0].enabled);
        assert!(!out[0].has_api_key);
    }
}

/// PATCH-shaped field diff for a [`SessionEntry`].
///
/// Every field follows the double-option convention documented on
/// [`SessionService::update_session_fields`]: outer `None` means
/// **leave alone**, `Some(None)` means **clear the override**, and
/// `Some(Some(value))` means **persist this value**.
///
/// Grouped into a struct so the router's 5-field call site and future
/// callers (Telegram `/model`, CLI, ...) are not exposed to a pile of
/// positional `Option<Option<String>>` arguments — a swap-typo waiting
/// to happen.
#[derive(Debug, Clone, Default)]
pub struct SessionPatch {
    /// New human-readable title.
    pub title:          Option<Option<String>>,
    /// New LLM model identifier.
    pub model:          Option<Option<String>>,
    /// New provider identifier paired with `model`.
    pub model_provider: Option<Option<String>>,
    /// New thinking-level override.
    pub thinking_level: Option<Option<ThinkingLevel>>,
    /// New system prompt override.
    pub system_prompt:  Option<Option<String>>,
}

/// Apply a [`SessionPatch`] to a [`SessionEntry`] in place, returning
/// `true` when any field actually changed.
///
/// Extracting the mutation into a free function keeps the per-field
/// branching trivially unit testable without spinning up tape storage
/// or a settings provider. The caller uses the boolean result to skip
/// the `updated_at` bump and the session-index write on a no-op PATCH
/// (e.g. a double-click on the "Use default" row re-sending `null` for
/// already-`None` fields).
fn apply_session_patch(session: &mut SessionEntry, patch: &SessionPatch) -> bool {
    /// Assign `new` to `slot` when the patch carries that field and the
    /// stored value actually differs. Returns whether a write happened
    /// so the caller can aggregate a single `changed` flag across all
    /// five fields.
    fn assign<T: Clone + PartialEq>(slot: &mut Option<T>, new: &Option<Option<T>>) -> bool {
        match new {
            Some(v) if slot != v => {
                *slot = v.clone();
                true
            }
            _ => false,
        }
    }

    // `|` (bitwise-or) instead of `||` is intentional: every field must
    // be evaluated so its write lands even when an earlier field already
    // flipped `changed` to true.
    assign(&mut session.title, &patch.title)
        | assign(&mut session.model, &patch.model)
        | assign(&mut session.model_provider, &patch.model_provider)
        | assign(&mut session.thinking_level, &patch.thinking_level)
        | assign(&mut session.system_prompt, &patch.system_prompt)
}

/// One matched message surfaced by
/// [`SessionService::search_sessions`].
///
/// The hit is a projection of a single [`TapEntry`] plus a pre-rendered
/// snippet so the web UI can render a search result list directly from
/// the JSON payload without re-reading the tape.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct SessionSearchHit {
    /// Session key (also the tape name) this message belongs to.
    pub session_key:   String,
    /// Display title of the session, falling back to `session_key` when
    /// no title is set. The UI should render this as the result's
    /// clickable label.
    pub session_title: String,
    /// HTML-escaped text with the first matched query token wrapped in
    /// `<mark>…</mark>`. Safe to insert into the DOM via `innerHTML`.
    pub snippet:       String,
    /// Role of the matched message: `"user"`, `"assistant"`, or
    /// `"other"` for system/tool/developer messages.
    pub role:          String,
    /// Wall-clock time of the underlying tape entry, in milliseconds
    /// since the Unix epoch.
    pub timestamp_ms:  i64,
    /// Monotonic tape-entry ID — exposed so the UI can deep-link into a
    /// specific message inside the session view.
    pub seq:           u64,
}

/// Response body for `GET /api/v1/chat/sessions/search`.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct SessionSearchResponse {
    /// Top-ranked hits, one per session.
    pub hits: Vec<SessionSearchHit>,
}

/// Central orchestrator for session-based AI chat.
///
/// `SessionService` ties together two concerns:
///
/// 1. **Session metadata** — CRUD operations on sessions and channel bindings,
///    delegated to a [`SessionIndexRef`] implementation.
/// 2. **Channel routing** — Mapping external messaging channels to internal
///    session keys via channel bindings.
///
/// Message persistence is handled by the tape subsystem ([`TapeService`]).
/// LLM execution has moved to the kernel path (`process_loop`).
///
/// The service is cheaply cloneable (`Arc`-wrapped internals) and safe to
/// share across axum handler tasks.
#[derive(Clone)]
pub struct SessionService {
    /// Tape-based session index for metadata.
    session_index:     SessionIndexRef,
    /// Tape service for append-only session recording.
    tape_service:      TapeService,
    /// Persisted per-turn execution traces. Used by
    /// [`Self::get_execution_trace`] to resolve a clicked assistant
    /// message's seq back to its owning turn's trace.
    trace_service:     TraceService,
    /// Cached catalog of models fetched from the LLM provider.
    model_catalog:     ModelCatalog,
    /// Settings provider for reading and writing flat KV settings.
    settings_provider: Arc<dyn SettingsProvider>,
}

impl SessionService {
    // -- full-text search ---------------------------------------------------

    /// Over-fetch multiplier for cross-tape search.
    ///
    /// The kernel returns ranked hits across every tape; we then collapse
    /// to one hit per session. Over-fetching leaves headroom after
    /// dedup so the response still carries up to `limit` distinct
    /// sessions when the top matches cluster inside a few long threads.
    const SEARCH_OVERFETCH_MULTIPLIER: usize = 3;

    /// Create a new chat service with the given dependencies.
    #[must_use]
    pub fn new(
        session_index: SessionIndexRef,
        tape_service: TapeService,
        trace_service: TraceService,
        settings_provider: Arc<dyn SettingsProvider>,
        model_lister: rara_kernel::llm::LlmModelListerRef,
    ) -> Self {
        Self {
            session_index,
            tape_service,
            trace_service,
            model_catalog: ModelCatalog::new(model_lister),
            settings_provider,
        }
    }

    // -- model catalog ------------------------------------------------------

    /// List available models from the configured provider. Favorites are
    /// marked and sorted to the top.
    pub async fn list_models(&self) -> Vec<ChatModel> {
        let favorites_json = self.settings_provider.get(keys::LLM_FAVORITE_MODELS).await;
        let favorites: Vec<String> = favorites_json
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_default();
        self.model_catalog.list_models(&favorites).await
    }

    /// Replace the user's favorite model list and persist to settings.
    pub async fn set_favorite_models(&self, ids: Vec<String>) -> Result<(), ChatError> {
        let json = serde_json::to_string(&ids).unwrap_or_default();
        self.settings_provider
            .set(keys::LLM_FAVORITE_MODELS, &json)
            .await
            .map_err(|e| ChatError::SessionError {
                message: format!("failed to update favorite models: {e}"),
            })?;
        Ok(())
    }

    /// List LLM providers derived from `llm.providers.<id>.*` settings,
    /// stripped of any sensitive fields. Only `api_key` presence is
    /// surfaced (as a boolean); actual key material never leaves the
    /// backend via this endpoint.
    pub async fn list_llm_providers(&self) -> Vec<ProviderInfo> {
        let all = self.settings_provider.list().await;
        collect_providers(&all)
    }

    // -- session CRUD -------------------------------------------------------

    /// Create a new session with optional overrides.
    ///
    /// The session key is generated (UUID).
    #[instrument(skip(self))]
    pub async fn create_session(
        &self,
        title: Option<String>,
        model: Option<String>,
        model_provider: Option<String>,
        thinking_level: Option<ThinkingLevel>,
        system_prompt: Option<String>,
    ) -> Result<SessionEntry, ChatError> {
        let now = Utc::now();
        let entry = SessionEntry {
            key: SessionKey::new(),
            title,
            model,
            model_provider,
            thinking_level,
            system_prompt,
            message_count: 0,
            preview: None,
            metadata: None,
            created_at: now,
            updated_at: now,
        };
        let created = self.session_index.create_session(&entry).await?;
        info!(key = %created.key, "session created");
        Ok(created)
    }

    /// List sessions ordered by most recently updated, with pagination.
    #[instrument(skip(self))]
    pub async fn list_sessions(
        &self,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> Result<Vec<SessionEntry>, ChatError> {
        let sessions = self
            .session_index
            .list_sessions(limit.unwrap_or(50), offset.unwrap_or(0))
            .await?;
        Ok(sessions)
    }

    /// Get a single session by key. Returns [`ChatError::SessionNotFound`]
    /// if the key does not exist.
    #[instrument(skip(self))]
    pub async fn get_session(&self, key: &SessionKey) -> Result<SessionEntry, ChatError> {
        self.session_index
            .get_session(key)
            .await?
            .ok_or_else(|| ChatError::SessionNotFound {
                key: key.to_string(),
            })
    }

    /// Partially update mutable fields of a session.
    ///
    /// The [`SessionPatch`] fields use the double-option convention so
    /// callers can separately express **leave alone** (outer `None`),
    /// **clear** (`Some(None)`) and **set** (`Some(Some(value))`). The
    /// clear variant is what lets a user drop a per-session pin and
    /// fall back to the admin `llm.default_provider`.
    ///
    /// Returns the session unchanged — without touching `updated_at` or
    /// writing to the session index — when the patch is a no-op, so
    /// repeat "Use default" clicks do not churn the list-order rank.
    #[instrument(skip(self, patch))]
    pub async fn update_session_fields(
        &self,
        key: &SessionKey,
        patch: SessionPatch,
    ) -> Result<SessionEntry, ChatError> {
        let mut session = self.get_session(key).await?;
        if !apply_session_patch(&mut session, &patch) {
            return Ok(session);
        }
        session.updated_at = Utc::now();
        let updated = self.session_index.update_session(&session).await?;
        info!(key = %key, "session fields updated");
        Ok(updated)
    }

    /// Delete a session.
    #[instrument(skip(self))]
    pub async fn delete_session(&self, key: &SessionKey) -> Result<(), ChatError> {
        self.session_index.delete_session(key).await?;
        info!(key = %key, "session deleted");
        Ok(())
    }

    // -- ensure session -----------------------------------------------------

    /// Ensure a session exists, creating it if it does not.
    ///
    /// Returns the existing or newly created [`SessionEntry`].
    #[instrument(skip(self))]
    pub async fn ensure_session(
        &self,
        key: &SessionKey,
        title: Option<&str>,
        model: Option<&str>,
        system_prompt: Option<&str>,
    ) -> Result<SessionEntry, ChatError> {
        match self.session_index.get_session(key).await? {
            Some(existing) => Ok(existing),
            None => {
                let now = Utc::now();
                let entry = SessionEntry {
                    key:            *key,
                    title:          title.map(ToOwned::to_owned),
                    model:          model.map(ToOwned::to_owned),
                    model_provider: None,
                    thinking_level: None,
                    system_prompt:  system_prompt.map(ToOwned::to_owned),
                    message_count:  0,
                    preview:        None,
                    metadata:       None,
                    created_at:     now,
                    updated_at:     now,
                };
                Ok(self.session_index.create_session(&entry).await?)
            }
        }
    }

    // -- messages (tape-backed) ---------------------------------------------

    /// List conversational messages for a session by reading tape entries.
    ///
    /// Only `Message`, `ToolCall`, and `ToolResult` entries are converted to
    /// [`ChatMessage`] structs. Non-conversational kinds are skipped.
    #[instrument(skip(self))]
    pub async fn list_messages(
        &self,
        key: &SessionKey,
        limit: usize,
    ) -> Result<Vec<ChatMessage>, ChatError> {
        let tape_name = key.to_string();
        let entries =
            self.tape_service
                .entries(&tape_name)
                .await
                .map_err(|e| ChatError::SessionError {
                    message: format!("failed to read tape: {e}"),
                })?;

        let messages = tap_entries_to_chat_messages(&entries);
        let total = messages.len();
        // Return the last `limit` messages (most recent).
        let start = total.saturating_sub(limit);
        Ok(messages[start..].to_vec())
    }

    /// Clear all tape entries for a session (reset the tape).
    #[instrument(skip(self))]
    pub async fn clear_messages(&self, key: &SessionKey) -> Result<(), ChatError> {
        let tape_name = key.to_string();
        self.tape_service
            .reset(&tape_name, false)
            .await
            .map_err(|e| ChatError::SessionError {
                message: format!("failed to clear tape: {e}"),
            })?;
        info!(key = %key, "messages cleared");
        Ok(())
    }

    // -- cascade trace ------------------------------------------------------

    /// Build a cascade execution trace for a specific turn in the session.
    ///
    /// The `message_seq` identifies the user message that starts the turn.
    /// All entries from that user message until the next user message (or
    /// end of tape) are collected and passed to the cascade builder.
    #[instrument(skip(self))]
    pub async fn get_cascade_trace(
        &self,
        key: &SessionKey,
        message_seq: usize,
    ) -> Result<CascadeTrace, ChatError> {
        let tape_name = key.to_string();
        let entries =
            self.tape_service
                .entries(&tape_name)
                .await
                .map_err(|e| ChatError::SessionError {
                    message: format!("failed to read tape: {e}"),
                })?;

        // Convert tape → chat messages so we can map the 1-based message_seq
        // back to the owning user-message turn.  The seq values in ChatMessage
        // can skip numbers (e.g. a ToolResult with N results increments seq
        // by N), so a direct index into tape entries is unreliable.
        let chat_msgs = tap_entries_to_chat_messages(&entries);

        let i_seq = message_seq as i64;
        // Find the last user message whose seq <= the clicked message_seq.
        let owning_user = chat_msgs
            .iter()
            .rfind(|m| m.role == MessageRole::User && m.seq <= i_seq);

        let Some(owner) = owning_user else {
            return Err(ChatError::InvalidRequest {
                message: format!("no user message found for seq {message_seq}"),
            });
        };

        // Determine the 0-based ordinal of this user message.
        let user_ordinal = chat_msgs
            .iter()
            .filter(|m| m.role == MessageRole::User)
            .position(|m| m.seq == owner.seq)
            .unwrap_or(0);

        // Extract the turn slice, then try the pre-built trace before
        // falling back to the post-hoc builder.
        let boundaries = find_turn_boundaries(&entries);
        let turn_entries = turn_slice(&entries, &boundaries, user_ordinal);

        if let Some(trace) = load_persisted_cascade(turn_entries) {
            return Ok(trace);
        }

        let message_id = format!("{}-{}", key, message_seq);
        let trace = build_cascade(turn_entries, &message_id);
        Ok(trace)
    }

    /// Fetch the persisted [`ExecutionTrace`] for a specific turn.
    ///
    /// The turn is identified by the seq of any message produced within
    /// it (most commonly the assistant reply the user clicked on). We
    /// find the owning user-message tape entry, read its
    /// `rara_message_id` metadata, and look the trace up via
    /// [`TraceService::find_trace_by_message_id`].
    ///
    /// Returns `InvalidRequest` when no user message precedes `seq` and
    /// `NotFound` when no trace has been persisted for the resolved
    /// turn (e.g. a legacy session recorded before trace storage existed).
    #[instrument(skip(self))]
    pub async fn get_execution_trace(
        &self,
        key: &SessionKey,
        message_seq: usize,
    ) -> Result<ExecutionTrace, ChatError> {
        let tape_name = key.to_string();
        let entries =
            self.tape_service
                .entries(&tape_name)
                .await
                .map_err(|e| ChatError::SessionError {
                    message: format!("failed to read tape: {e}"),
                })?;

        // Walk the tape mirroring `tap_entries_to_chat_messages`'s seq
        // counter so we can correlate `message_seq` back to the specific
        // user-message TapEntry. We keep that entry's `metadata`
        // (which is where `rara_message_id` is recorded) rather than
        // re-deriving it — the kernel writes it at turn start and it
        // uniquely keys the persisted trace row.
        let i_seq = message_seq as i64;
        let mut seq: i64 = 0;
        let mut last_user_entry: Option<&TapEntry> = None;
        for entry in &entries {
            match entry.kind {
                TapEntryKind::Message => {
                    if let Ok(msg) = serde_json::from_value::<Message>(entry.payload.clone()) {
                        seq += 1;
                        if seq > i_seq {
                            break;
                        }
                        if matches!(msg.role, Role::User) {
                            last_user_entry = Some(entry);
                        }
                    }
                }
                TapEntryKind::ToolCall | TapEntryKind::ToolResult => {
                    seq += 1;
                    if seq > i_seq {
                        break;
                    }
                }
                _ => {}
            }
        }

        let Some(user_entry) = last_user_entry else {
            return Err(ChatError::InvalidRequest {
                message: format!("no user message found for seq {message_seq}"),
            });
        };

        let rara_message_id = user_entry
            .metadata
            .as_ref()
            .and_then(|m| m.get("rara_message_id"))
            .and_then(Value::as_str)
            .ok_or_else(|| ChatError::NotFound {
                message: format!(
                    "user message at seq {message_seq} has no rara_message_id metadata"
                ),
            })?;

        let trace = self
            .trace_service
            .find_trace_by_message_id(rara_message_id)
            .await
            .map_err(|e| ChatError::SessionError {
                message: format!("failed to query execution trace: {e}"),
            })?;

        trace.map(|(_, t)| t).ok_or_else(|| ChatError::NotFound {
            message: format!("no execution trace recorded for message {rara_message_id}"),
        })
    }

    /// Search every session's tape for messages matching `query`.
    ///
    /// Issues a single FTS query across all tapes via
    /// [`TapeService::search_across_tapes`], then keeps only the
    /// highest-ranked hit per session before clamping to `limit`. One
    /// hit per session keeps the result list varied: the underlying FTS
    /// ranker tends to cluster many hits inside a long thread, which
    /// would otherwise drown out other sessions.
    ///
    /// An empty or whitespace-only `query` short-circuits to an empty
    /// response — this is treated as "user cleared the search box", not
    /// as a validation error.
    #[instrument(skip(self))]
    pub async fn search_sessions(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<SessionSearchResponse, ChatError> {
        let trimmed = query.trim();
        if trimmed.is_empty() || limit == 0 {
            return Ok(SessionSearchResponse { hits: Vec::new() });
        }

        // Over-fetch so per-session dedup still yields up to `limit`
        // distinct sessions when the top matches cluster in a few tapes.
        let fetch_limit = limit.saturating_mul(Self::SEARCH_OVERFETCH_MULTIPLIER);

        // Degrade gracefully: on tape-level failure we surface an empty
        // result rather than a 500, matching the original per-session
        // loop's "skip on error" semantics.
        let ranked = match self
            .tape_service
            .search_across_tapes(trimmed, fetch_limit)
            .await
        {
            Ok(hits) => hits,
            Err(e) => {
                tracing::warn!(error = %e, "cross-tape search failed");
                return Ok(SessionSearchResponse { hits: Vec::new() });
            }
        };

        // Cache session lookups so we only pay one index hit per tape.
        let mut session_cache: std::collections::HashMap<String, Option<SessionEntry>> =
            std::collections::HashMap::new();
        let mut seen_sessions: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut hits: Vec<SessionSearchHit> = Vec::new();

        for TapeSearchHit { entry, tape_name } in ranked {
            if hits.len() >= limit {
                break;
            }
            if !seen_sessions.insert(tape_name.clone()) {
                continue;
            }

            let session = match session_cache.get(&tape_name) {
                Some(cached) => cached.clone(),
                None => {
                    let key = match SessionKey::try_from_raw(&tape_name) {
                        Ok(k) => k,
                        Err(e) => {
                            tracing::debug!(
                                tape = %tape_name,
                                error = %e,
                                "skipping non-session tape"
                            );
                            session_cache.insert(tape_name.clone(), None);
                            continue;
                        }
                    };
                    let fetched = self.session_index.get_session(&key).await.ok().flatten();
                    session_cache.insert(tape_name.clone(), fetched.clone());
                    fetched
                }
            };
            let Some(session) = session else { continue };

            if let Some(hit) = project_search_hit(&entry, &session, trimmed) {
                hits.push(hit);
            }
        }

        Ok(SessionSearchResponse { hits })
    }

    // -- channel bindings ---------------------------------------------------

    /// Bind an external channel (e.g. Telegram chat) to a session key.
    ///
    /// `thread_id` associates the binding with a specific forum topic when
    /// present.
    #[instrument(skip(self))]
    pub async fn bind_channel(
        &self,
        channel_type: ChannelType,
        chat_id: String,
        session_key: SessionKey,
        thread_id: Option<&str>,
    ) -> Result<ChannelBinding, ChatError> {
        let now = Utc::now();
        let binding = ChannelBinding {
            channel_type,
            chat_id,
            thread_id: thread_id.map(str::to_owned),
            session_key,
            created_at: now,
            updated_at: now,
        };
        let result = self.session_index.bind_channel(&binding).await?;
        Ok(result)
    }

    /// Look up which session an external channel is currently bound to.
    ///
    /// `thread_id` narrows the lookup to a specific forum topic (Telegram
    /// supergroup threads).  Pass `None` for non-forum contexts.
    #[instrument(skip(self))]
    pub async fn get_channel_session(
        &self,
        channel_type: ChannelType,
        chat_id: &str,
        thread_id: Option<&str>,
    ) -> Result<Option<ChannelBinding>, ChatError> {
        let binding = self
            .session_index
            .get_channel_binding(channel_type, chat_id, thread_id)
            .await?;
        Ok(binding)
    }
}

impl std::fmt::Debug for SessionService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionService").finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// Tape → ChatMessage conversion
// ---------------------------------------------------------------------------

/// Convert tape entries into a flat list of [`ChatMessage`] structs.
///
/// Mirrors the logic in `memory/context.rs` but targets the channel-layer
/// `ChatMessage` type instead of `llm::Message`.
fn tap_entries_to_chat_messages(entries: &[TapEntry]) -> Vec<ChatMessage> {
    let mut messages = Vec::new();
    let mut seq: i64 = 0;
    let mut pending_calls: Vec<(String, String)> = Vec::new(); // (id, name)

    for entry in entries {
        match entry.kind {
            TapEntryKind::Message => {
                if let Ok(msg) = serde_json::from_value::<Message>(entry.payload.clone()) {
                    seq += 1;
                    let role = match msg.role {
                        Role::System | Role::Developer => MessageRole::System,
                        Role::User => MessageRole::User,
                        Role::Assistant => MessageRole::Assistant,
                        Role::Tool => MessageRole::Tool,
                    };
                    // Preserve multimodal content (images) via serde round-trip
                    // between llm::MessageContent and channel::types::MessageContent
                    // (both share the same serde format).
                    let content: MessageContent = serde_json::to_value(&msg.content)
                        .and_then(|v| serde_json::from_value(v))
                        .unwrap_or_else(|_| MessageContent::Text(msg.content.as_text().to_owned()));
                    let tool_calls: Vec<ChannelToolCall> = msg
                        .tool_calls
                        .iter()
                        .map(|tc| ChannelToolCall {
                            id:        tc.id.clone().into(),
                            name:      tc.name.clone().into(),
                            arguments: serde_json::from_str(&tc.arguments)
                                .unwrap_or(Value::String(tc.arguments.clone())),
                        })
                        .collect();
                    messages.push(ChatMessage {
                        seq,
                        role,
                        content,
                        tool_calls,
                        tool_call_id: msg.tool_call_id.clone(),
                        tool_name: None,
                        created_at: entry.timestamp,
                    });
                }
            }
            TapEntryKind::ToolCall => {
                pending_calls.clear();
                if let Some(calls) = entry.payload.get("calls").and_then(Value::as_array) {
                    let mut tc_list = Vec::new();
                    for call in calls {
                        let id = call
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned();
                        let func = call.get("function").and_then(Value::as_object);
                        let name = func
                            .and_then(|f| f.get("name"))
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned();
                        let arguments = func
                            .and_then(|f| f.get("arguments"))
                            .and_then(Value::as_str)
                            .unwrap_or("{}");
                        let args_val: Value = serde_json::from_str(arguments)
                            .unwrap_or(Value::String(arguments.to_owned()));
                        pending_calls.push((id.clone(), name.clone()));
                        tc_list.push(ChannelToolCall {
                            id:        id.into(),
                            name:      name.into(),
                            arguments: args_val,
                        });
                    }
                    seq += 1;
                    messages.push(ChatMessage {
                        seq,
                        role: MessageRole::Assistant,
                        content: MessageContent::Text(String::new()),
                        tool_calls: tc_list,
                        tool_call_id: None,
                        tool_name: None,
                        created_at: entry.timestamp,
                    });
                }
            }
            TapEntryKind::ToolResult => {
                if let Some(results) = entry.payload.get("results").and_then(Value::as_array) {
                    for (i, result) in results.iter().enumerate() {
                        let content_str = match result {
                            Value::String(s) => s.clone(),
                            other => serde_json::to_string(other).unwrap_or_default(),
                        };
                        let (call_id, tool_name) =
                            pending_calls.get(i).cloned().unwrap_or_default();
                        seq += 1;
                        messages.push(ChatMessage {
                            seq,
                            role: MessageRole::ToolResult,
                            content: MessageContent::Text(content_str),
                            tool_calls: Vec::new(),
                            tool_call_id: if call_id.is_empty() {
                                None
                            } else {
                                Some(call_id)
                            },
                            tool_name: if tool_name.is_empty() {
                                None
                            } else {
                                Some(tool_name)
                            },
                            created_at: entry.timestamp,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    messages
}

/// Map a kernel [`Role`] to the short, lowercase string the web UI
/// expects on a search hit.
fn role_label(role: Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
        // System / Developer / Tool messages are rare hits but can surface
        // when the user searches for content embedded in a tool result;
        // collapse the distinction so the UI has a single "other" bucket.
        _ => "other",
    }
}

/// Extract the plain text body of a [`Message`] for snippet generation.
fn message_plain_text(msg: &Message) -> String { msg.content.as_text().to_owned() }

/// Project a matched [`TapEntry`] into a [`SessionSearchHit`].
///
/// Returns `None` when the entry is not a decodable `Message` — we
/// deliberately skip `ToolCall` / `ToolResult` / system metadata entries
/// here because the search endpoint targets conversational content. A
/// sibling user / assistant entry with overlapping text will usually
/// be ranked alongside the skipped entry and picked up instead.
fn project_search_hit(
    entry: &TapEntry,
    session: &SessionEntry,
    query: &str,
) -> Option<SessionSearchHit> {
    if !matches!(entry.kind, TapEntryKind::Message) {
        return None;
    }
    let msg: Message = serde_json::from_value(entry.payload.clone()).ok()?;
    let text = message_plain_text(&msg);
    let snippet = crate::chat::snippet::build_snippet(&text, query);
    let session_key = session.key.to_string();
    let session_title = session
        .title
        .as_ref()
        .filter(|t| !t.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| session_key.clone());
    Some(SessionSearchHit {
        session_key,
        session_title,
        snippet,
        role: role_label(msg.role).to_owned(),
        timestamp_ms: entry.timestamp.as_millisecond(),
        seq: entry.id,
    })
}

#[cfg(test)]
mod session_patch_tests {
    use chrono::Utc;
    use rara_sessions::types::{SessionEntry, SessionKey, ThinkingLevel};

    use super::{SessionPatch, apply_session_patch};

    fn sample_session() -> SessionEntry {
        let now = Utc::now();
        SessionEntry {
            key:            SessionKey::new(),
            title:          Some("existing title".to_owned()),
            model:          Some("kimi-k2.5".to_owned()),
            model_provider: Some("kimi".to_owned()),
            thinking_level: Some(ThinkingLevel::Medium),
            system_prompt:  Some("hello".to_owned()),
            message_count:  0,
            preview:        None,
            metadata:       None,
            created_at:     now,
            updated_at:     now,
        }
    }

    #[test]
    fn absent_fields_leave_session_untouched() {
        let mut session = sample_session();
        let before = session.clone();
        let changed = apply_session_patch(&mut session, &SessionPatch::default());
        assert!(!changed, "all-absent patch must report no-op");
        assert_eq!(session.title, before.title);
        assert_eq!(session.model, before.model);
        assert_eq!(session.model_provider, before.model_provider);
        assert_eq!(session.thinking_level, before.thinking_level);
        assert_eq!(session.system_prompt, before.system_prompt);
    }

    #[test]
    fn explicit_null_clears_model_override() {
        // This is the #1569 case: a session pinned to `kimi` is reset so
        // the admin's `llm.default_provider` can take effect on the next
        // turn.
        let mut session = sample_session();
        let patch = SessionPatch {
            model: Some(None),
            model_provider: Some(None),
            thinking_level: Some(None),
            ..Default::default()
        };
        let changed = apply_session_patch(&mut session, &patch);
        assert!(changed);
        assert!(session.model.is_none());
        assert!(session.model_provider.is_none());
        assert!(session.thinking_level.is_none());
        // Fields not in the patch are preserved.
        assert_eq!(session.title.as_deref(), Some("existing title"));
        assert_eq!(session.system_prompt.as_deref(), Some("hello"));
    }

    #[test]
    fn partial_patch_clears_only_targeted_field() {
        // Only `model` is cleared; `model_provider` and every other
        // field must remain untouched.
        let mut session = sample_session();
        let patch = SessionPatch {
            model: Some(None),
            ..Default::default()
        };
        let changed = apply_session_patch(&mut session, &patch);
        assert!(changed);
        assert!(session.model.is_none());
        assert_eq!(session.model_provider.as_deref(), Some("kimi"));
        assert_eq!(session.thinking_level, Some(ThinkingLevel::Medium));
        assert_eq!(session.title.as_deref(), Some("existing title"));
        assert_eq!(session.system_prompt.as_deref(), Some("hello"));
    }

    #[test]
    fn all_fields_cleared_in_one_call() {
        let mut session = sample_session();
        let patch = SessionPatch {
            title:          Some(None),
            model:          Some(None),
            model_provider: Some(None),
            thinking_level: Some(None),
            system_prompt:  Some(None),
        };
        let changed = apply_session_patch(&mut session, &patch);
        assert!(changed);
        assert!(session.title.is_none());
        assert!(session.model.is_none());
        assert!(session.model_provider.is_none());
        assert!(session.thinking_level.is_none());
        assert!(session.system_prompt.is_none());
    }

    #[test]
    fn some_value_overwrites_field() {
        let mut session = sample_session();
        let patch = SessionPatch {
            title:          Some(Some("renamed".to_owned())),
            model:          Some(Some("gpt-4o".to_owned())),
            model_provider: Some(Some("openai".to_owned())),
            thinking_level: Some(Some(ThinkingLevel::High)),
            system_prompt:  Some(Some("new prompt".to_owned())),
        };
        let changed = apply_session_patch(&mut session, &patch);
        assert!(changed);
        assert_eq!(session.title.as_deref(), Some("renamed"));
        assert_eq!(session.model.as_deref(), Some("gpt-4o"));
        assert_eq!(session.model_provider.as_deref(), Some("openai"));
        assert_eq!(session.thinking_level, Some(ThinkingLevel::High));
        assert_eq!(session.system_prompt.as_deref(), Some("new prompt"));
    }

    #[test]
    fn setting_same_value_is_a_noop() {
        // Patching `model_provider` to the value it already holds must
        // report `false` so the caller can skip the index write.
        let mut session = sample_session();
        let before = session.clone();
        let patch = SessionPatch {
            model_provider: Some(Some("kimi".to_owned())),
            ..Default::default()
        };
        let changed = apply_session_patch(&mut session, &patch);
        assert!(!changed);
        assert_eq!(session.model_provider, before.model_provider);
    }
}

#[cfg(test)]
mod update_request_deserialize_tests {
    use crate::chat::UpdateSessionRequest;

    #[test]
    fn absent_fields_deserialize_to_outer_none() {
        let req: UpdateSessionRequest = serde_json::from_str("{}").expect("parse");
        assert!(req.title.is_none());
        assert!(req.model.is_none());
        assert!(req.model_provider.is_none());
        assert!(req.thinking_level.is_none());
        assert!(req.system_prompt.is_none());
    }

    #[test]
    fn explicit_null_model_deserializes_to_some_none() {
        let req: UpdateSessionRequest =
            serde_json::from_str(r#"{"model": null, "model_provider": null}"#).expect("parse");
        assert_eq!(req.model, Some(None));
        assert_eq!(req.model_provider, Some(None));
        assert!(req.thinking_level.is_none());
    }

    #[test]
    fn explicit_value_model_deserializes_to_some_some() {
        let req: UpdateSessionRequest =
            serde_json::from_str(r#"{"model": "gpt-4o", "thinking_level": "high"}"#)
                .expect("parse");
        assert_eq!(req.model, Some(Some("gpt-4o".to_owned())));
        assert_eq!(req.thinking_level, Some(Some("high".to_owned())));
    }

    #[test]
    fn title_triple_absent_null_value() {
        let absent: UpdateSessionRequest = serde_json::from_str("{}").expect("parse");
        assert!(absent.title.is_none());
        let null: UpdateSessionRequest = serde_json::from_str(r#"{"title": null}"#).expect("parse");
        assert_eq!(null.title, Some(None));
        let value: UpdateSessionRequest =
            serde_json::from_str(r#"{"title": "hi"}"#).expect("parse");
        assert_eq!(value.title, Some(Some("hi".to_owned())));
    }

    #[test]
    fn model_provider_triple_absent_null_value() {
        let absent: UpdateSessionRequest = serde_json::from_str("{}").expect("parse");
        assert!(absent.model_provider.is_none());
        let null: UpdateSessionRequest =
            serde_json::from_str(r#"{"model_provider": null}"#).expect("parse");
        assert_eq!(null.model_provider, Some(None));
        let value: UpdateSessionRequest =
            serde_json::from_str(r#"{"model_provider": "openai"}"#).expect("parse");
        assert_eq!(value.model_provider, Some(Some("openai".to_owned())));
    }

    #[test]
    fn thinking_level_triple_absent_null_value() {
        let absent: UpdateSessionRequest = serde_json::from_str("{}").expect("parse");
        assert!(absent.thinking_level.is_none());
        let null: UpdateSessionRequest =
            serde_json::from_str(r#"{"thinking_level": null}"#).expect("parse");
        assert_eq!(null.thinking_level, Some(None));
        let value: UpdateSessionRequest =
            serde_json::from_str(r#"{"thinking_level": "medium"}"#).expect("parse");
        assert_eq!(value.thinking_level, Some(Some("medium".to_owned())));
    }

    #[test]
    fn system_prompt_triple_absent_null_value() {
        let absent: UpdateSessionRequest = serde_json::from_str("{}").expect("parse");
        assert!(absent.system_prompt.is_none());
        let null: UpdateSessionRequest =
            serde_json::from_str(r#"{"system_prompt": null}"#).expect("parse");
        assert_eq!(null.system_prompt, Some(None));
        let value: UpdateSessionRequest =
            serde_json::from_str(r#"{"system_prompt": "you are..."}"#).expect("parse");
        assert_eq!(value.system_prompt, Some(Some("you are...".to_owned())));
    }
}

#[cfg(test)]
mod search_sessions_tests {
    use std::{collections::HashMap, sync::Arc};

    use async_trait::async_trait;
    use chrono::Utc;
    use rara_domain_shared::settings::SettingsProvider;
    use rara_kernel::{
        llm::{LlmModelLister, ModelInfo},
        memory::{FileTapeStore, TapeService},
        session::{SessionIndex, SessionKey, test_utils::InMemorySessionIndex},
        trace::TraceService,
    };
    use rara_sessions::types::SessionEntry;
    use serde_json::json;
    use tokio::sync::OnceCell;

    use super::SessionService;

    /// Process-wide fixture for the search tests.
    ///
    /// Building a `SessionService + FTS-backed TapeService` costs ~1s per
    /// fixture (diesel pool migration + tape store init). The four tests in
    /// this module are independent at the FTS-marker level (`zephyrrising`,
    /// `pingpong-token`, `clampme`, `hello`), so a single shared fixture is
    /// sufficient — each test still uses fresh `SessionKey::new()` UUIDs and
    /// asserts via marker-scoped queries, never on global counts.
    static SHARED: OnceCell<SharedFixture> = OnceCell::const_new();

    struct SharedFixture {
        service:  SessionService,
        sessions: Arc<InMemorySessionIndex>,
        // Keep the tape directory alive for the entire process — tape files
        // are written under this path and the FTS index references them.
        _tmp:     tempfile::TempDir,
    }

    async fn shared_fixture() -> &'static SharedFixture {
        SHARED
            .get_or_init(|| async {
                let tmp = tempfile::tempdir().expect("tempdir");
                let (service, sessions) = build_service_with_fts(tmp.path()).await;
                SharedFixture {
                    service,
                    sessions,
                    _tmp: tmp,
                }
            })
            .await
    }

    struct StubSettings;

    #[async_trait]
    impl SettingsProvider for StubSettings {
        async fn get(&self, _key: &str) -> Option<String> { None }

        async fn set(&self, _key: &str, _value: &str) -> anyhow::Result<()> { Ok(()) }

        async fn delete(&self, _key: &str) -> anyhow::Result<()> { Ok(()) }

        async fn list(&self) -> HashMap<String, String> { HashMap::new() }

        async fn batch_update(
            &self,
            _patches: HashMap<String, Option<String>>,
        ) -> anyhow::Result<()> {
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

    async fn build_service_with_fts(
        dir: &std::path::Path,
    ) -> (SessionService, Arc<InMemorySessionIndex>) {
        let pool = rara_kernel::testing::build_memory_diesel_pools().await;

        let store = FileTapeStore::new(dir, dir).await.unwrap();
        let tape_service = TapeService::with_fts(store, pool.clone());
        let sessions: Arc<InMemorySessionIndex> = Arc::new(InMemorySessionIndex::new());
        let trace_service = TraceService::new(pool);
        let service = SessionService::new(
            sessions.clone(),
            tape_service,
            trace_service,
            Arc::new(StubSettings),
            Arc::new(StubModelLister),
        );
        (service, sessions)
    }

    async fn register_session(index: &InMemorySessionIndex, key: &SessionKey, title: &str) {
        let now = Utc::now();
        let entry = SessionEntry {
            key:            key.clone(),
            title:          Some(title.to_owned()),
            model:          None,
            model_provider: None,
            thinking_level: None,
            system_prompt:  None,
            message_count:  0,
            preview:        None,
            metadata:       None,
            created_at:     now,
            updated_at:     now,
        };
        index.create_session(&entry).await.expect("create session");
    }

    #[tokio::test]
    async fn empty_query_returns_no_hits() {
        let fx = shared_fixture().await;
        let service = &fx.service;
        let sessions = &fx.sessions;

        let key = SessionKey::new();
        register_session(sessions, &key, "session one").await;
        service
            .tape_service()
            .append_message(&key.to_string(), json!({"content": "hello"}), None)
            .await
            .unwrap();

        for q in ["", "   ", "\t\n"] {
            let resp = service.search_sessions(q, 20).await.unwrap();
            assert!(resp.hits.is_empty(), "empty query should yield no hits");
        }
    }

    #[tokio::test]
    async fn attribution_across_many_sessions() {
        let fx = shared_fixture().await;
        let service = &fx.service;
        let sessions = &fx.sessions;

        let marker = "zephyrrising";
        let mut keys = Vec::new();
        for i in 0..12 {
            let key = SessionKey::new();
            register_session(sessions, &key, &format!("session-{i}")).await;
            service
                .tape_service()
                .append_message(
                    &key.to_string(),
                    json!({"role": "user", "content": format!("note {i}: {marker} body text")}),
                    None,
                )
                .await
                .unwrap();
            keys.push(key);
        }

        let resp = service.search_sessions(marker, 20).await.unwrap();
        assert_eq!(resp.hits.len(), 12);

        // Dedup invariant: every hit belongs to a distinct session.
        let unique: std::collections::HashSet<_> =
            resp.hits.iter().map(|h| h.session_key.clone()).collect();
        assert_eq!(unique.len(), resp.hits.len());

        // Attribution: every hit's session_key must match one of the
        // sessions we registered.
        let registered: std::collections::HashSet<String> =
            keys.iter().map(ToString::to_string).collect();
        for hit in &resp.hits {
            assert!(
                registered.contains(&hit.session_key),
                "unexpected session {}",
                hit.session_key
            );
            assert!(hit.snippet.contains("<mark>"));
        }
    }

    #[tokio::test]
    async fn dedup_keeps_one_hit_per_session() {
        let fx = shared_fixture().await;
        let service = &fx.service;
        let sessions = &fx.sessions;

        let key = SessionKey::new();
        register_session(sessions, &key, "busy session").await;
        for i in 0..5 {
            service
                .tape_service()
                .append_message(
                    &key.to_string(),
                    json!({"role": "user", "content": format!("pingpong-token body {i}")}),
                    None,
                )
                .await
                .unwrap();
        }

        let resp = service.search_sessions("pingpong-token", 20).await.unwrap();
        assert_eq!(resp.hits.len(), 1, "one tape should yield one hit");
        assert_eq!(resp.hits[0].session_key, key.to_string());
    }

    #[tokio::test]
    async fn limit_clamps_output() {
        let fx = shared_fixture().await;
        let service = &fx.service;
        let sessions = &fx.sessions;

        for i in 0..8 {
            let key = SessionKey::new();
            register_session(sessions, &key, &format!("s{i}")).await;
            service
                .tape_service()
                .append_message(
                    &key.to_string(),
                    json!({"role": "user", "content": format!("clampme body {i}")}),
                    None,
                )
                .await
                .unwrap();
        }

        let resp = service.search_sessions("clampme", 3).await.unwrap();
        assert_eq!(resp.hits.len(), 3);
    }

    impl SessionService {
        /// Test-only accessor to populate tapes before searching.
        fn tape_service(&self) -> &TapeService { &self.tape_service }
    }
}
