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
//! [`MemoryManager`] is the single entry point for all memory operations:
//! - **search** — parallel query across mem0 + Hindsight, fused with RRF.
//! - **write_note** — persist a Markdown note to Memos.
//! - **reflect_on_exchange** — post-conversation reflection that fans out to
//!   all three backends in parallel.
//! - **get_user_profile** — retrieve structured facts from mem0.
//! - **deep_recall** — trigger Hindsight's deep reasoning.

use crate::error::MemoryResult;
use crate::fusion::reciprocal_rank_fusion;
use crate::hindsight_client::HindsightClient;
use crate::mem0_client::{Mem0Client, Mem0Memory, Mem0Message};
use crate::memos_client::MemosClient;

/// Which backend a [`SearchResult`] originated from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemorySource {
    Mem0,
    Hindsight,
    Memos,
}

/// A single search result from the unified memory layer.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub id: String,
    pub source: MemorySource,
    pub content: String,
    /// Fused relevance score (higher is better).
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
    /// Create a new [`MemoryManager`].
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
    /// Reciprocal Rank Fusion.
    pub async fn search(&self, query: &str, limit: usize) -> MemoryResult<Vec<SearchResult>> {
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
    /// Searches with a broad query to surface all known facts.
    pub async fn get_user_profile(&self) -> MemoryResult<Vec<Mem0Memory>> {
        self.mem0
            .search("user profile preferences facts", &self.user_id, 50)
            .await
    }

    /// Trigger Hindsight's deep reasoning over the memory bank.
    pub async fn deep_recall(&self, query: &str) -> MemoryResult<String> {
        self.hindsight.reflect(query).await
    }
}
