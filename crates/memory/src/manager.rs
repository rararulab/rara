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

//! Unified memory manager that orchestrates mem0, Memos, and Hindsight.
//!
//! [`MemoryManager`] is the **single entry point** for all memory operations
//! in the agent system. Upper layers (tools, orchestrator) never call the
//! individual clients directly — they always go through this facade.
//!
//! # Trigger Timing
//!
//! Each backend has a distinct trigger policy:
//!
//! - **mem0** — fires at session-end (via [`consolidate_session`]) or explicit
//!   fact addition (via [`add_fact`]). Never per-turn.
//! - **Memos** — only written via the explicit `memory_write` tool
//!   ([`write_note`]). No automatic writes.
//! - **Hindsight** — fires at session-end (via [`consolidate_session`]) or
//!   explicit fact addition (via [`add_fact`]). Never per-turn.
//!
//! # Operations
//!
//! | Method                   | Backends touched     | Purpose                                        |
//! |--------------------------|----------------------|-------------------------------------------------|
//! | [`search`]               | mem0 + Hindsight (‖) | Parallel semantic search, fused via RRF          |
//! | [`write_note`]           | Memos                | Persist a tagged Markdown note                   |
//! | [`consolidate_session`]  | mem0 + Hindsight (‖) | Batch session exchanges at session-end           |
//! | [`add_fact`]             | mem0 + Hindsight (‖) | Store a single explicit fact                     |
//! | [`get_user_profile`]     | mem0                 | Retrieve structured user facts                   |
//! | [`deep_recall`]          | Hindsight            | Personality-conditioned deep reasoning            |
//!
//! **(‖)** = backends are queried in parallel via `tokio::join!`.
//!
//! # Error Handling
//!
//! - [`search`], [`consolidate_session`], and [`add_fact`] are **best-effort**:
//!   individual backend failures are logged as warnings but do not propagate as
//!   errors. This ensures the agent remains functional even if one backend is
//!   down.
//! - [`write_note`], [`get_user_profile`], and [`deep_recall`] propagate errors
//!   directly since they target a single backend.
//!
//! [`search`]: MemoryManager::search
//! [`write_note`]: MemoryManager::write_note
//! [`consolidate_session`]: MemoryManager::consolidate_session
//! [`add_fact`]: MemoryManager::add_fact
//! [`get_user_profile`]: MemoryManager::get_user_profile
//! [`deep_recall`]: MemoryManager::deep_recall

use crate::{
    error::MemoryResult,
    fusion::reciprocal_rank_fusion,
    hindsight_client::HindsightClient,
    mem0_client::{Mem0Client, Mem0Memory, Mem0Message},
    memos_client::MemosClient,
};

/// Which backend a [`SearchResult`] originated from.
///
/// Used by the tool layer to annotate search results so the agent (and
/// the user) can tell where a piece of information came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemorySource {
    /// Structured fact from mem0 (state layer).
    Mem0,
    /// Memory from Hindsight's 4-network model (learning layer).
    Hindsight,
    /// Markdown note from Memos (storage layer).
    Memos,
}

impl std::fmt::Display for MemorySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Mem0 => write!(f, "mem0"),
            Self::Hindsight => write!(f, "hindsight"),
            Self::Memos => write!(f, "memos"),
        }
    }
}

/// A single search result from the unified memory layer.
///
/// After [RRF fusion](crate::fusion::reciprocal_rank_fusion), the `score`
/// field reflects the fused rank score (not the original backend score).
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// Backend-specific identifier (mem0 memory ID or Hindsight record ID).
    pub id:      String,
    /// Which backend this result originated from.
    pub source:  MemorySource,
    /// The memory content (fact text, note content, or recalled passage).
    pub content: String,
    /// Fused relevance score (higher is better). After RRF, items appearing
    /// in multiple backends receive a boosted score.
    pub score:   f64,
}

/// High-level memory orchestrator backed by three external services.
///
/// - **mem0** — structured fact extraction and semantic search.
/// - **Memos** — persistent Markdown note storage.
/// - **Hindsight** — 4-network (world/experience/opinion/observation)
///   retain/recall/reflect.
pub struct MemoryManager {
    mem0:      Mem0Client,
    memos:     MemosClient,
    hindsight: HindsightClient,
    user_id:   String,
}

impl MemoryManager {
    /// Create a new [`MemoryManager`] from pre-configured backend clients.
    ///
    /// The `user_id` is used as the scoping key for mem0 — all memories are
    /// stored and searched under this user. Typically `"default"` for a
    /// single-user deployment.
    pub fn new(
        mem0: Mem0Client,
        memos: MemosClient,
        hindsight: HindsightClient,
        user_id: String,
    ) -> Self {
        Self {
            mem0,
            memos,
            hindsight,
            user_id,
        }
    }

    /// Access the mem0 client.
    pub(crate) fn mem0(&self) -> &Mem0Client { &self.mem0 }

    /// Access the Memos client.
    pub(crate) fn memos(&self) -> &MemosClient { &self.memos }

    /// Access the Hindsight client.
    pub(crate) fn hindsight(&self) -> &HindsightClient { &self.hindsight }

    /// Search across mem0 and Hindsight in parallel, then fuse results with
    /// Reciprocal Rank Fusion (RRF).
    ///
    /// Each backend is asked for `max(limit * 3, 10)` candidates to give RRF
    /// enough signal for re-ranking. If one backend fails, the other's results
    /// are still returned (with a warning logged).
    ///
    /// Returns at most `limit` results sorted by descending fused score.
    pub async fn search(&self, query: &str, limit: usize) -> MemoryResult<Vec<SearchResult>> {
        // Over-fetch so RRF has enough candidates to produce a good ranking.
        let fetch_limit = (limit * 3).max(10);

        let (mem0_res, hindsight_res) = tokio::join!(
            self.mem0.search(query, &self.user_id, fetch_limit),
            self.hindsight.recall(query, fetch_limit),
        );

        let mut result_sets = Vec::new();

        match mem0_res {
            Ok(memories) => {
                let set: Vec<SearchResult> = memories
                    .into_iter()
                    .map(|m| SearchResult {
                        id:      m.id,
                        source:  MemorySource::Mem0,
                        content: m.memory,
                        score:   m.score.unwrap_or(0.0),
                    })
                    .collect();
                result_sets.push(set);
            }
            Err(e) => {
                tracing::warn!(error = %e, "mem0 search failed, skipping");
            }
        }

        match hindsight_res {
            Ok(memories) => {
                let set: Vec<SearchResult> = memories
                    .into_iter()
                    .map(|m| SearchResult {
                        id:      m.id,
                        source:  MemorySource::Hindsight,
                        content: m.content,
                        score:   m.score,
                    })
                    .collect();
                result_sets.push(set);
            }
            Err(e) => {
                tracing::warn!(error = %e, "hindsight recall failed, skipping");
            }
        }

        Ok(reciprocal_rank_fusion(result_sets, limit, 60.0))
    }

    /// Write a Markdown note to Memos.
    ///
    /// Tags are prepended as `#tag` lines at the top of the content.
    /// Returns the memo resource name (e.g. `"memos/123"`).
    pub async fn write_note(&self, content: &str, tags: &[&str]) -> MemoryResult<String> {
        let mut body = String::new();
        for tag in tags {
            body.push('#');
            body.push_str(tag);
            body.push(' ');
        }
        if !tags.is_empty() {
            body.push('\n');
        }
        body.push_str(content);

        let entry = self.memos.create_memo(&body, "PRIVATE").await?;
        Ok(entry.name)
    }

    /// Consolidate a completed session's exchanges into long-term memory.
    ///
    /// Called at session boundaries (when inactivity threshold is exceeded),
    /// **not** on every conversation turn. Batches all exchanges into:
    ///
    /// 1. **mem0** — one `add_memories` call with all user/assistant messages.
    /// 2. **Hindsight** — one `retain` call with the full session text.
    ///
    /// Memos is **not** touched — notes are only written via the explicit
    /// `memory_write` tool.
    ///
    /// This method is best-effort: partial backend failures are logged as
    /// warnings but do not propagate as errors.
    pub async fn consolidate_session(&self, exchanges: &[(String, String)]) -> MemoryResult<()> {
        if exchanges.is_empty() {
            return Ok(());
        }

        // Build mem0 message list from all exchanges.
        let messages: Vec<Mem0Message> = exchanges
            .iter()
            .flat_map(|(user, assistant)| {
                vec![
                    Mem0Message {
                        role:    "user".to_owned(),
                        content: user.clone(),
                    },
                    Mem0Message {
                        role:    "assistant".to_owned(),
                        content: assistant.clone(),
                    },
                ]
            })
            .collect();

        // Build full session text for Hindsight retain.
        let full_text: String = exchanges
            .iter()
            .map(|(user, assistant)| format!("User: {user}\nAssistant: {assistant}"))
            .collect::<Vec<_>>()
            .join("\n\n");

        let (mem0_res, hindsight_res) = tokio::join!(
            self.mem0.add_memories(messages, &self.user_id),
            self.hindsight.retain(&full_text),
        );

        // Log warnings for partial failures but don't fail the whole operation.
        if let Err(e) = mem0_res {
            tracing::warn!(error = %e, "mem0 add_memories failed during session consolidation");
        }
        if let Err(e) = hindsight_res {
            tracing::warn!(error = %e, "hindsight retain failed during session consolidation");
        }

        Ok(())
    }

    /// Store a single explicit fact in mem0 and Hindsight.
    ///
    /// Unlike [`consolidate_session`](Self::consolidate_session) which batches
    /// an entire session, this stores one piece of information immediately.
    /// Used by the `memory_add_fact` tool.
    ///
    /// Best-effort: partial backend failures are logged as warnings.
    pub async fn add_fact(&self, content: &str) -> MemoryResult<()> {
        let messages = vec![Mem0Message {
            role:    "user".to_owned(),
            content: content.to_owned(),
        }];

        let (mem0_res, hindsight_res) = tokio::join!(
            self.mem0.add_memories(messages, &self.user_id),
            self.hindsight.retain(content),
        );

        if let Err(e) = mem0_res {
            tracing::warn!(error = %e, "mem0 add_memories failed during add_fact");
        }
        if let Err(e) = hindsight_res {
            tracing::warn!(error = %e, "hindsight retain failed during add_fact");
        }

        Ok(())
    }

    /// Retrieve the user's structured fact profile from mem0.
    ///
    /// Issues a broad semantic search (`"user profile preferences facts"`)
    /// to surface all known facts about the user. Returns up to 50 memories.
    ///
    /// These facts are typically injected into the system prompt so the
    /// agent has persistent awareness of user preferences and context.
    pub async fn get_user_profile(&self) -> MemoryResult<Vec<Mem0Memory>> {
        self.mem0
            .search("user profile preferences facts", &self.user_id, 50)
            .await
    }

    /// Trigger Hindsight's deep reasoning (reflect) over the memory bank.
    ///
    /// Unlike [`search`](Self::search) which returns raw memory fragments,
    /// deep recall asks Hindsight to **synthesize** an answer by reasoning
    /// across all four networks (world, experience, opinion, observation).
    ///
    /// Returns a free-form text response from Hindsight's reflect endpoint.
    pub async fn deep_recall(&self, query: &str) -> MemoryResult<String> {
        self.hindsight.reflect(query).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        hindsight_client::HindsightClient, mem0_client::Mem0Client, memos_client::MemosClient,
    };

    /// Build a MemoryManager from environment variables.
    /// Returns None if required env vars are not set.
    fn manager() -> Option<MemoryManager> {
        let mem0_url = std::env::var("MEM0_BASE_URL").ok()?;
        let memos_url = std::env::var("MEMOS_BASE_URL").ok()?;
        let memos_token = std::env::var("MEMOS_TOKEN").unwrap_or_default();
        let hindsight_url = std::env::var("HINDSIGHT_BASE_URL").ok()?;
        let hindsight_bank =
            std::env::var("HINDSIGHT_BANK_ID").unwrap_or_else(|_| "integration-test".into());

        let mem0 = Mem0Client::new(mem0_url);
        let memos = MemosClient::new(memos_url, memos_token);
        let hindsight = HindsightClient::new(hindsight_url, hindsight_bank);
        Some(MemoryManager::new(
            mem0,
            memos,
            hindsight,
            "integration-test".into(),
        ))
    }

    // -- unit tests (no infra needed) --

    #[test]
    fn memory_source_display() {
        assert_eq!(MemorySource::Mem0.to_string(), "mem0");
        assert_eq!(MemorySource::Hindsight.to_string(), "hindsight");
        assert_eq!(MemorySource::Memos.to_string(), "memos");
    }

    // -- integration tests --

    #[tokio::test]
    #[ignore = "requires all 3 memory services (set MEM0_BASE_URL, MEMOS_BASE_URL, \
                HINDSIGHT_BASE_URL)"]
    async fn search_returns_fused_results() {
        let mm = manager().expect("memory service env vars required");
        let results = mm
            .search("programming languages", 10)
            .await
            .expect("search failed");
        println!("search returned {} fused results", results.len());
        for r in &results {
            println!(
                "  [{:.4}] [{}] {}",
                r.score,
                r.source,
                &r.content[..r.content.len().min(80)]
            );
        }
    }

    #[tokio::test]
    #[ignore = "requires Memos service (set MEMOS_BASE_URL, MEMOS_TOKEN)"]
    async fn write_note_and_verify() {
        let mm = manager().expect("memory service env vars required");

        let name = mm
            .write_note(
                "integration test note from MemoryManager",
                &["rara-test", "integration"],
            )
            .await
            .expect("write_note failed");
        println!("write_note returned: {name}");
        assert!(name.starts_with("memos/"));

        // Clean up: extract id and delete via memos client directly
        let id = name.strip_prefix("memos/").unwrap_or(&name);
        let memos_url = std::env::var("MEMOS_BASE_URL").unwrap();
        let memos_token = std::env::var("MEMOS_TOKEN").unwrap_or_default();
        let memos = MemosClient::new(memos_url, memos_token);
        memos.delete_memo(id).await.expect("cleanup delete failed");
    }

    #[tokio::test]
    #[ignore = "requires mem0 + Hindsight services"]
    async fn consolidate_session_tolerates_partial_failures() {
        let mm = manager().expect("memory service env vars required");
        let exchanges = vec![
            (
                "I'm looking for Rust backend jobs in Shanghai".to_owned(),
                "I'll help you search for Rust backend positions in Shanghai.".to_owned(),
            ),
            (
                "What about remote positions?".to_owned(),
                "Let me also check remote Rust backend listings for you.".to_owned(),
            ),
        ];
        // This should succeed even if some backends have issues
        mm.consolidate_session(&exchanges)
            .await
            .expect("consolidate_session failed");
        println!("consolidate_session completed successfully");
    }

    #[tokio::test]
    #[ignore = "requires mem0 + Hindsight services"]
    async fn add_fact_tolerates_partial_failures() {
        let mm = manager().expect("memory service env vars required");
        mm.add_fact("The user prefers Rust backend roles in Shanghai or remote.")
            .await
            .expect("add_fact failed");
        println!("add_fact completed successfully");
    }

    #[tokio::test]
    #[ignore = "requires mem0 service (set MEM0_BASE_URL)"]
    async fn get_user_profile() {
        let mm = manager().expect("memory service env vars required");
        let profile = mm
            .get_user_profile()
            .await
            .expect("get_user_profile failed");
        println!("user profile has {} facts", profile.len());
        for fact in &profile {
            println!("  - {}", fact.memory);
        }
    }

    #[tokio::test]
    #[ignore = "requires Hindsight service (set HINDSIGHT_BASE_URL)"]
    async fn deep_recall() {
        let mm = manager().expect("memory service env vars required");
        let response = mm
            .deep_recall("What are the user's career goals?")
            .await
            .expect("deep_recall failed");
        println!("deep_recall: {}", &response[..response.len().min(200)]);
        assert!(!response.is_empty());
    }
}
