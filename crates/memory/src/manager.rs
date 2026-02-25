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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::MemoryError;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Start 3 MockServers and build a MemoryManager wired to them.
    async fn setup_manager() -> (MemoryManager, MockServer, MockServer, MockServer) {
        let mem0_server = MockServer::start().await;
        let memos_server = MockServer::start().await;
        let hindsight_server = MockServer::start().await;

        let mem0 = Mem0Client::new(mem0_server.uri());
        let memos = MemosClient::new(memos_server.uri(), "test-token".into());
        let hindsight =
            crate::hindsight_client::HindsightClient::new(hindsight_server.uri(), "test-bank".into());
        let manager = MemoryManager::new(mem0, memos, hindsight, "test-user".into());

        (manager, mem0_server, memos_server, hindsight_server)
    }

    fn mem0_search_response(items: &[(&str, &str, f64)]) -> serde_json::Value {
        let arr: Vec<serde_json::Value> = items
            .iter()
            .map(|(id, memory, score)| {
                serde_json::json!({
                    "id": id,
                    "memory": memory,
                    "user_id": "test-user",
                    "score": score,
                    "created_at": "2025-01-01T00:00:00Z",
                    "updated_at": "2025-01-01T00:00:00Z"
                })
            })
            .collect();
        serde_json::Value::Array(arr)
    }

    fn hindsight_recall_response(items: &[(&str, &str, f64)]) -> serde_json::Value {
        let arr: Vec<serde_json::Value> = items
            .iter()
            .map(|(id, content, score)| {
                serde_json::json!({
                    "id": id,
                    "content": content,
                    "network": "world",
                    "score": score
                })
            })
            .collect();
        serde_json::Value::Array(arr)
    }

    fn sample_memo_entry() -> serde_json::Value {
        serde_json::json!({
            "name": "memos/1",
            "uid": "uid-123",
            "content": "test content",
            "visibility": "PRIVATE",
            "pinned": false,
            "createTime": "2025-01-01T00:00:00Z",
            "updateTime": "2025-01-01T00:00:00Z"
        })
    }

    // ---- search (7) ----

    #[tokio::test]
    async fn search_both_succeed() {
        let (manager, mem0_server, _memos_server, hindsight_server) = setup_manager().await;

        // mem0 returns x, y
        Mock::given(method("POST"))
            .and(path("/v1/memories/search/"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(&mem0_search_response(&[("x", "fact x", 0.9), ("y", "fact y", 0.8)])),
            )
            .mount(&mem0_server)
            .await;

        // hindsight returns y, z
        Mock::given(method("POST"))
            .and(path("/api/v1/banks/test-bank/recall"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(&hindsight_recall_response(&[("y", "recall y", 0.9), ("z", "recall z", 0.7)])),
            )
            .mount(&hindsight_server)
            .await;

        let results = manager.search("query", 10).await.unwrap();
        // "y" appears in both lists, so it should have the highest RRF score.
        assert!(!results.is_empty());
        assert_eq!(results[0].id, "y");
    }

    #[tokio::test]
    async fn search_mem0_fails() {
        let (manager, mem0_server, _memos_server, hindsight_server) = setup_manager().await;

        Mock::given(method("POST"))
            .and(path("/v1/memories/search/"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mem0_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/api/v1/banks/test-bank/recall"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(&hindsight_recall_response(&[("z", "recall z", 0.9)])),
            )
            .mount(&hindsight_server)
            .await;

        let results = manager.search("query", 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "z");
        assert_eq!(results[0].source, MemorySource::Hindsight);
    }

    #[tokio::test]
    async fn search_hindsight_fails() {
        let (manager, mem0_server, _memos_server, hindsight_server) = setup_manager().await;

        Mock::given(method("POST"))
            .and(path("/v1/memories/search/"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(&mem0_search_response(&[("x", "fact x", 0.9)])),
            )
            .mount(&mem0_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/api/v1/banks/test-bank/recall"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&hindsight_server)
            .await;

        let results = manager.search("query", 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "x");
        assert_eq!(results[0].source, MemorySource::Mem0);
    }

    #[tokio::test]
    async fn search_both_fail() {
        let (manager, mem0_server, _memos_server, hindsight_server) = setup_manager().await;

        Mock::given(method("POST"))
            .and(path("/v1/memories/search/"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mem0_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/api/v1/banks/test-bank/recall"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&hindsight_server)
            .await;

        let results = manager.search("query", 10).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn search_respects_limit() {
        let (manager, mem0_server, _memos_server, hindsight_server) = setup_manager().await;

        Mock::given(method("POST"))
            .and(path("/v1/memories/search/"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(&mem0_search_response(&[
                    ("a", "a", 0.9),
                    ("b", "b", 0.8),
                    ("c", "c", 0.7),
                ])),
            )
            .mount(&mem0_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/api/v1/banks/test-bank/recall"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(&hindsight_recall_response(&[
                    ("d", "d", 0.9),
                    ("e", "e", 0.8),
                ])),
            )
            .mount(&hindsight_server)
            .await;

        let results = manager.search("query", 2).await.unwrap();
        assert!(results.len() <= 2);
    }

    #[tokio::test]
    async fn search_overfetch() {
        let (manager, mem0_server, _memos_server, hindsight_server) = setup_manager().await;

        // When limit=1, fetch_limit = max(1*3, 10) = 10
        // Verify mem0 receives top_k=10
        let expected_body = serde_json::json!({
            "query": "test",
            "user_id": "test-user",
            "top_k": 10,
        });

        Mock::given(method("POST"))
            .and(path("/v1/memories/search/"))
            .and(body_json(&expected_body))
            .respond_with(ResponseTemplate::new(200).set_body_json(&serde_json::json!([])))
            .mount(&mem0_server)
            .await;

        // Hindsight also receives top_k=10
        let expected_hindsight_body = serde_json::json!({
            "query": "test",
            "top_k": 10,
        });

        Mock::given(method("POST"))
            .and(path("/api/v1/banks/test-bank/recall"))
            .and(body_json(&expected_hindsight_body))
            .respond_with(ResponseTemplate::new(200).set_body_json(&serde_json::json!([])))
            .mount(&hindsight_server)
            .await;

        let results = manager.search("test", 1).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn search_parallel() {
        let (manager, mem0_server, _memos_server, hindsight_server) = setup_manager().await;

        // Both backends respond with 100ms delay.
        Mock::given(method("POST"))
            .and(path("/v1/memories/search/"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(&serde_json::json!([]))
                    .set_delay(std::time::Duration::from_millis(100)),
            )
            .mount(&mem0_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/api/v1/banks/test-bank/recall"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(&serde_json::json!([]))
                    .set_delay(std::time::Duration::from_millis(100)),
            )
            .mount(&hindsight_server)
            .await;

        let start = std::time::Instant::now();
        let _results = manager.search("test", 10).await.unwrap();
        let elapsed = start.elapsed();

        // If parallel, total time should be ~100ms, not ~200ms.
        assert!(
            elapsed < std::time::Duration::from_millis(250),
            "search took {:?}, expected < 250ms (parallel)",
            elapsed
        );
    }

    // ---- write_note (4) ----

    #[tokio::test]
    async fn write_note_no_tags() {
        let (manager, _mem0_server, memos_server, _hindsight_server) = setup_manager().await;

        Mock::given(method("POST"))
            .and(path("/api/v1/memos"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&sample_memo_entry()))
            .mount(&memos_server)
            .await;

        let name = manager.write_note("plain content", &[]).await.unwrap();
        assert_eq!(name, "memos/1");
    }

    #[tokio::test]
    async fn write_note_with_tags() {
        let (manager, _mem0_server, memos_server, _hindsight_server) = setup_manager().await;

        // When tags are ["daily", "log"], the body sent to Memos should start with "#daily #log \n"
        let expected_body = serde_json::json!({
            "content": "#daily #log \nsome content",
            "visibility": "PRIVATE",
        });

        Mock::given(method("POST"))
            .and(path("/api/v1/memos"))
            .and(body_json(&expected_body))
            .respond_with(ResponseTemplate::new(200).set_body_json(&sample_memo_entry()))
            .mount(&memos_server)
            .await;

        let name = manager
            .write_note("some content", &["daily", "log"])
            .await
            .unwrap();
        assert_eq!(name, "memos/1");
    }

    #[tokio::test]
    async fn write_note_returns_name() {
        let (manager, _mem0_server, memos_server, _hindsight_server) = setup_manager().await;

        let mut entry = sample_memo_entry();
        entry["name"] = serde_json::json!("memos/999");

        Mock::given(method("POST"))
            .and(path("/api/v1/memos"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&entry))
            .mount(&memos_server)
            .await;

        let name = manager.write_note("content", &[]).await.unwrap();
        assert_eq!(name, "memos/999");
    }

    #[tokio::test]
    async fn write_note_error() {
        let (manager, _mem0_server, memos_server, _hindsight_server) = setup_manager().await;

        Mock::given(method("POST"))
            .and(path("/api/v1/memos"))
            .respond_with(ResponseTemplate::new(500).set_body_string("error"))
            .mount(&memos_server)
            .await;

        let err = manager.write_note("content", &[]).await.unwrap_err();
        assert!(matches!(err, MemoryError::Memos { .. }));
    }

    // ---- reflect_on_exchange (4) ----

    #[tokio::test]
    async fn reflect_all_succeed() {
        let (manager, mem0_server, memos_server, hindsight_server) = setup_manager().await;

        Mock::given(method("POST"))
            .and(path("/v1/memories/"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(&serde_json::json!({"results": []})),
            )
            .expect(1)
            .mount(&mem0_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/api/v1/banks/test-bank/retain"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&hindsight_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/api/v1/memos"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&sample_memo_entry()))
            .expect(1)
            .mount(&memos_server)
            .await;

        manager
            .reflect_on_exchange("hello", "world")
            .await
            .unwrap();
        // Expectations are verified on drop.
    }

    #[tokio::test]
    async fn reflect_mem0_fails() {
        let (manager, mem0_server, memos_server, hindsight_server) = setup_manager().await;

        Mock::given(method("POST"))
            .and(path("/v1/memories/"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mem0_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/api/v1/banks/test-bank/retain"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&hindsight_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/api/v1/memos"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&sample_memo_entry()))
            .mount(&memos_server)
            .await;

        // Should still return Ok even though mem0 failed.
        manager
            .reflect_on_exchange("hello", "world")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn reflect_all_fail() {
        let (manager, mem0_server, memos_server, hindsight_server) = setup_manager().await;

        Mock::given(method("POST"))
            .and(path("/v1/memories/"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mem0_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/api/v1/banks/test-bank/retain"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&hindsight_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/api/v1/memos"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&memos_server)
            .await;

        // Should still return Ok even when all backends fail.
        manager
            .reflect_on_exchange("hello", "world")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn reflect_correct_payloads() {
        let (manager, mem0_server, memos_server, hindsight_server) = setup_manager().await;

        // mem0 should receive the messages array
        let expected_mem0_body = serde_json::json!({
            "messages": [
                {"role": "user", "content": "hi there"},
                {"role": "assistant", "content": "hello back"}
            ],
            "user_id": "test-user",
        });

        Mock::given(method("POST"))
            .and(path("/v1/memories/"))
            .and(body_json(&expected_mem0_body))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(&serde_json::json!({"results": []})),
            )
            .mount(&mem0_server)
            .await;

        // hindsight should receive the retain content
        let expected_hindsight_body = serde_json::json!({
            "content": "User: hi there\nAssistant: hello back",
        });

        Mock::given(method("POST"))
            .and(path("/api/v1/banks/test-bank/retain"))
            .and(body_json(&expected_hindsight_body))
            .respond_with(ResponseTemplate::new(200))
            .mount(&hindsight_server)
            .await;

        // memos should receive the log content
        let expected_memos_body = serde_json::json!({
            "content": "## Exchange Log\n\nUser: hi there\nAssistant: hello back",
            "visibility": "PRIVATE",
        });

        Mock::given(method("POST"))
            .and(path("/api/v1/memos"))
            .and(body_json(&expected_memos_body))
            .respond_with(ResponseTemplate::new(200).set_body_json(&sample_memo_entry()))
            .mount(&memos_server)
            .await;

        manager
            .reflect_on_exchange("hi there", "hello back")
            .await
            .unwrap();
    }

    // ---- get_user_profile (3) ----

    #[tokio::test]
    async fn get_user_profile_success() {
        let (manager, mem0_server, _memos_server, _hindsight_server) = setup_manager().await;

        Mock::given(method("POST"))
            .and(path("/v1/memories/search/"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(&mem0_search_response(&[
                    ("p1", "likes rust", 0.9),
                    ("p2", "lives in shanghai", 0.8),
                ])),
            )
            .mount(&mem0_server)
            .await;

        let profile = manager.get_user_profile().await.unwrap();
        assert_eq!(profile.len(), 2);
        assert_eq!(profile[0].id, "p1");
        assert_eq!(profile[1].id, "p2");
    }

    #[tokio::test]
    async fn get_user_profile_broad_query() {
        let (manager, mem0_server, _memos_server, _hindsight_server) = setup_manager().await;

        let expected_body = serde_json::json!({
            "query": "user profile preferences facts",
            "user_id": "test-user",
            "top_k": 50,
        });

        Mock::given(method("POST"))
            .and(path("/v1/memories/search/"))
            .and(body_json(&expected_body))
            .respond_with(ResponseTemplate::new(200).set_body_json(&serde_json::json!([])))
            .mount(&mem0_server)
            .await;

        let profile = manager.get_user_profile().await.unwrap();
        assert!(profile.is_empty());
    }

    #[tokio::test]
    async fn get_user_profile_error() {
        let (manager, mem0_server, _memos_server, _hindsight_server) = setup_manager().await;

        Mock::given(method("POST"))
            .and(path("/v1/memories/search/"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mem0_server)
            .await;

        let err = manager.get_user_profile().await.unwrap_err();
        assert!(matches!(err, MemoryError::Mem0 { .. }));
    }

    // ---- deep_recall (2) ----

    #[tokio::test]
    async fn deep_recall_success() {
        let (manager, _mem0_server, _memos_server, hindsight_server) = setup_manager().await;

        Mock::given(method("POST"))
            .and(path("/api/v1/banks/test-bank/reflect"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(&serde_json::json!({"response": "deep insight"})),
            )
            .mount(&hindsight_server)
            .await;

        let answer = manager.deep_recall("deep question").await.unwrap();
        assert_eq!(answer, "deep insight");
    }

    #[tokio::test]
    async fn deep_recall_error() {
        let (manager, _mem0_server, _memos_server, hindsight_server) = setup_manager().await;

        Mock::given(method("POST"))
            .and(path("/api/v1/banks/test-bank/reflect"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&hindsight_server)
            .await;

        let err = manager.deep_recall("query").await.unwrap_err();
        assert!(matches!(err, MemoryError::Hindsight { .. }));
    }

    // ---- display (3) ----

    #[test]
    fn memory_source_display_mem0() {
        assert_eq!(MemorySource::Mem0.to_string(), "mem0");
    }

    #[test]
    fn memory_source_display_hindsight() {
        assert_eq!(MemorySource::Hindsight.to_string(), "hindsight");
    }

    #[test]
    fn memory_source_display_memos() {
        assert_eq!(MemorySource::Memos.to_string(), "memos");
    }
}
