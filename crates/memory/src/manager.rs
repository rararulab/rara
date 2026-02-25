// Copyright 2025 Crrow
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
//! # Operations
//!
//! | Method                  | Backends touched         | Purpose                                        |
//! |-------------------------|--------------------------|-------------------------------------------------|
//! | [`search`]              | mem0 + Hindsight (‖)     | Parallel semantic search, fused via RRF          |
//! | [`write_note`]          | Memos                    | Persist a tagged Markdown note                   |
//! | [`reflect_on_exchange`] | mem0 + Hindsight + Memos (‖) | Post-turn reflection across all three        |
//! | [`get_user_profile`]    | mem0                     | Retrieve structured user facts                   |
//! | [`deep_recall`]         | Hindsight                | Personality-conditioned deep reasoning            |
//!
//! **(‖)** = backends are queried in parallel via `tokio::join!`.
//!
//! # Error Handling
//!
//! - [`search`] and [`reflect_on_exchange`] are **best-effort**: individual
//!   backend failures are logged as warnings but do not propagate as errors.
//!   This ensures the agent remains functional even if one backend is down.
//! - [`write_note`], [`get_user_profile`], and [`deep_recall`] propagate
//!   errors directly since they target a single backend.
//!
//! [`search`]: MemoryManager::search
//! [`write_note`]: MemoryManager::write_note
//! [`reflect_on_exchange`]: MemoryManager::reflect_on_exchange
//! [`get_user_profile`]: MemoryManager::get_user_profile
//! [`deep_recall`]: MemoryManager::deep_recall

use crate::error::MemoryResult;
use crate::fusion::reciprocal_rank_fusion;
use crate::hindsight_client::HindsightClient;
use crate::mem0_client::{Mem0Client, Mem0Memory, Mem0Message};
use crate::memos_client::MemosClient;

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
    pub id: String,
    /// Which backend this result originated from.
    pub source: MemorySource,
    /// The memory content (fact text, note content, or recalled passage).
    pub content: String,
    /// Fused relevance score (higher is better). After RRF, items appearing
    /// in multiple backends receive a boosted score.
    pub score: f64,
}

/// High-level memory orchestrator backed by three external services.
///
/// - **mem0** — structured fact extraction and semantic search.
/// - **Memos** — persistent Markdown note storage.
/// - **Hindsight** — 4-network (world/experience/opinion/observation)
///   retain/recall/reflect.
pub struct MemoryManager {
    mem0: Mem0Client,
    memos: MemosClient,
    hindsight: HindsightClient,
    user_id: String,
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
                        id: m.id,
                        source: MemorySource::Mem0,
                        content: m.memory,
                        score: m.score.unwrap_or(0.0),
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
                        id: m.id,
                        source: MemorySource::Hindsight,
                        content: m.content,
                        score: m.score,
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

    /// Post-conversation reflection: fan out to all three backends in parallel.
    ///
    /// 1. **mem0** — extract facts from the exchange.
    /// 2. **Hindsight** — retain the exchange for 4-network recall.
    /// 3. **Memos** — append a timestamped daily log entry.
    pub async fn reflect_on_exchange(
        &self,
        user_text: &str,
        assistant_text: &str,
    ) -> MemoryResult<()> {
        let messages = vec![
            Mem0Message {
                role: "user".to_owned(),
                content: user_text.to_owned(),
            },
            Mem0Message {
                role: "assistant".to_owned(),
                content: assistant_text.to_owned(),
            },
        ];

        let exchange_text = format!("User: {user_text}\nAssistant: {assistant_text}");
        let log_content = format!("## Exchange Log\n\n{exchange_text}");

        let (mem0_res, hindsight_res, memos_res) = tokio::join!(
            self.mem0.add_memories(messages, &self.user_id),
            self.hindsight.retain(&exchange_text),
            self.memos.create_memo(&log_content, "PRIVATE"),
        );

        // Log warnings for partial failures but don't fail the whole operation.
        if let Err(e) = mem0_res {
            tracing::warn!(error = %e, "mem0 add_memories failed during reflect");
        }
        if let Err(e) = hindsight_res {
            tracing::warn!(error = %e, "hindsight retain failed during reflect");
        }
        if let Err(e) = memos_res {
            tracing::warn!(error = %e, "memos daily log failed during reflect");
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
