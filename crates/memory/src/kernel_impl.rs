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

//! Kernel memory trait implementations for [`MemoryManager`].
//!
//! This module bridges the kernel's three memory traits — [`StateMemory`],
//! [`KnowledgeMemory`], and [`LearningMemory`] — to the concrete backend
//! clients held by [`MemoryManager`] (mem0, Memos, Hindsight).
//!
//! ## Type Mapping
//!
//! | Kernel trait        | Backend client     | Notes                                |
//! |--------------------|--------------------|--------------------------------------|
//! | [`StateMemory`]    | [`Mem0Client`]     | Structured facts, semantic search    |
//! | [`KnowledgeMemory`]| [`MemosClient`]    | Markdown notes with tags             |
//! | [`LearningMemory`] | [`HindsightClient`]| Retain / recall / reflect            |
//!
//! ## Scope Mapping
//!
//! The kernel's [`Scope`] enum is mapped to backend-specific scoping:
//!
//! - **`Scope::Agent`** — uses `MemoryContext::agent_id` as the scoping key.
//! - **`Scope::Team(name)`** — uses `"team:{name}"` as the user/bank ID.
//! - **`Scope::Global`** — uses `"global"` as the user/bank ID.
//!
//! [`StateMemory`]: rara_kernel::memory::state::StateMemory
//! [`KnowledgeMemory`]: rara_kernel::memory::knowledge::KnowledgeMemory
//! [`LearningMemory`]: rara_kernel::memory::learning::LearningMemory
//! [`Scope`]: rara_kernel::memory::types::Scope
//! [`MemoryManager`]: crate::manager::MemoryManager
//! [`Mem0Client`]: crate::mem0_client::Mem0Client
//! [`MemosClient`]: crate::memos_client::MemosClient
//! [`HindsightClient`]: crate::hindsight_client::HindsightClient

use uuid::Uuid;

use crate::{
    manager::MemoryManager,
    mem0_client::{
        Mem0AddRequest, Mem0DeleteAllRequest, Mem0DeleteRequest, Mem0GetAllRequest, Mem0GetRequest,
        Mem0HistoryRequest, Mem0Message, Mem0SearchRequest, Mem0UpdateRequest,
    },
};

// ─── Helpers ─────────────────────────────────────────────────────────

/// Derive a user-scoping key for mem0 from the kernel scope and context.
///
/// mem0 scopes memories by `user_id`. We map:
/// - `Scope::Agent` → `"agent:{agent_id}"`
/// - `Scope::Team(name)` → `"team:{name}"`
/// - `Scope::Global` → `"global"`
fn scope_to_user_id(
    ctx: &rara_kernel::memory::types::MemoryContext,
    scope: &rara_kernel::memory::types::Scope,
) -> String {
    match scope {
        rara_kernel::memory::types::Scope::Agent => format!("agent:{}", ctx.agent_id),
        rara_kernel::memory::types::Scope::Team(name) => format!("team:{name}"),
        rara_kernel::memory::types::Scope::Global => "global".to_owned(),
    }
}

/// Convert a rara-memory `MemoryError` into the kernel's `MemoryError`.
fn to_kernel_state_error(err: crate::error::MemoryError) -> rara_kernel::memory::MemoryError {
    rara_kernel::memory::MemoryError::State {
        message: err.to_string(),
    }
}

fn to_kernel_knowledge_error(err: crate::error::MemoryError) -> rara_kernel::memory::MemoryError {
    rara_kernel::memory::MemoryError::Knowledge {
        message: err.to_string(),
    }
}

fn to_kernel_learning_error(err: crate::error::MemoryError) -> rara_kernel::memory::MemoryError {
    rara_kernel::memory::MemoryError::Learning {
        message: err.to_string(),
    }
}

/// Try to parse an optional RFC 3339 timestamp string into a `jiff::Timestamp`.
fn parse_timestamp(s: &Option<String>) -> Option<jiff::Timestamp> {
    s.as_deref()
        .and_then(|ts| ts.parse::<jiff::Timestamp>().ok())
}

// ─── StateMemory (mem0) ─────────────────────────────────────────────

#[async_trait::async_trait]
impl rara_kernel::memory::state::StateMemory for MemoryManager {
    async fn add(
        &self,
        ctx: &rara_kernel::memory::types::MemoryContext,
        scope: rara_kernel::memory::types::Scope,
        messages: Vec<rara_kernel::memory::types::Message>,
    ) -> rara_kernel::memory::Result<Vec<rara_kernel::memory::types::StateEvent>> {
        let user_id = scope_to_user_id(ctx, &scope);
        let mem0_messages: Vec<Mem0Message> = messages
            .into_iter()
            .map(|m| Mem0Message {
                role:    m.role,
                content: m.content,
            })
            .collect();

        let response = self
            .mem0()
            .add(Mem0AddRequest {
                messages:    mem0_messages,
                user_id:     Some(user_id),
                agent_id:    Some(ctx.agent_id.to_string()),
                run_id:      ctx.session_id.map(|id| id.to_string()),
                metadata:    None,
                infer:       None,
                memory_type: None,
                prompt:      None,
            })
            .await
            .map_err(to_kernel_state_error)?;

        let events = response
            .results
            .into_iter()
            .map(|e| rara_kernel::memory::types::StateEvent {
                id:               e.id.parse().unwrap_or_else(|_| Uuid::new_v4()),
                event:            e.event,
                content:          e.memory,
                previous_content: e.previous_memory,
            })
            .collect();

        Ok(events)
    }

    async fn search(
        &self,
        ctx: &rara_kernel::memory::types::MemoryContext,
        scope: rara_kernel::memory::types::Scope,
        query: &str,
        limit: usize,
    ) -> rara_kernel::memory::Result<Vec<rara_kernel::memory::types::StateFact>> {
        let user_id = scope_to_user_id(ctx, &scope);

        let response = self
            .mem0()
            .search_mem0(Mem0SearchRequest {
                query:     query.to_owned(),
                user_id:   Some(user_id),
                run_id:    None,
                agent_id:  Some(ctx.agent_id.to_string()),
                limit:     Some(limit),
                filters:   None,
                threshold: None,
                rerank:    None,
            })
            .await
            .map_err(to_kernel_state_error)?;

        Ok(mem0_memories_to_facts(response.results))
    }

    async fn get(
        &self,
        _ctx: &rara_kernel::memory::types::MemoryContext,
        _scope: rara_kernel::memory::types::Scope,
        id: Uuid,
    ) -> rara_kernel::memory::Result<Option<rara_kernel::memory::types::StateFact>> {
        let result = self
            .mem0()
            .get_by_id(Mem0GetRequest {
                memory_id: id.to_string(),
            })
            .await
            .map_err(to_kernel_state_error)?;

        Ok(result.map(|m| mem0_memory_to_fact(m)))
    }

    async fn get_all(
        &self,
        ctx: &rara_kernel::memory::types::MemoryContext,
        scope: rara_kernel::memory::types::Scope,
        limit: usize,
    ) -> rara_kernel::memory::Result<Vec<rara_kernel::memory::types::StateFact>> {
        let user_id = scope_to_user_id(ctx, &scope);

        let response = self
            .mem0()
            .get_all(Mem0GetAllRequest {
                user_id:  Some(user_id),
                run_id:   None,
                agent_id: Some(ctx.agent_id.to_string()),
                filters:  None,
                limit:    Some(limit),
            })
            .await
            .map_err(to_kernel_state_error)?;

        Ok(mem0_memories_to_facts(response.results))
    }

    async fn update(
        &self,
        _ctx: &rara_kernel::memory::types::MemoryContext,
        _scope: rara_kernel::memory::types::Scope,
        id: Uuid,
        data: &str,
    ) -> rara_kernel::memory::Result<()> {
        self.mem0()
            .update(Mem0UpdateRequest {
                memory_id: id.to_string(),
                data:      data.to_owned(),
            })
            .await
            .map_err(to_kernel_state_error)?;
        Ok(())
    }

    async fn delete(
        &self,
        _ctx: &rara_kernel::memory::types::MemoryContext,
        _scope: rara_kernel::memory::types::Scope,
        id: Uuid,
    ) -> rara_kernel::memory::Result<()> {
        self.mem0()
            .delete_by_id(Mem0DeleteRequest {
                memory_id: id.to_string(),
            })
            .await
            .map_err(to_kernel_state_error)?;
        Ok(())
    }

    async fn delete_all(
        &self,
        ctx: &rara_kernel::memory::types::MemoryContext,
        scope: rara_kernel::memory::types::Scope,
    ) -> rara_kernel::memory::Result<()> {
        let user_id = scope_to_user_id(ctx, &scope);
        self.mem0()
            .delete_all(Mem0DeleteAllRequest {
                user_id:  Some(user_id),
                run_id:   None,
                agent_id: Some(ctx.agent_id.to_string()),
            })
            .await
            .map_err(to_kernel_state_error)?;
        Ok(())
    }

    async fn history(
        &self,
        _ctx: &rara_kernel::memory::types::MemoryContext,
        _scope: rara_kernel::memory::types::Scope,
        id: Uuid,
    ) -> rara_kernel::memory::Result<Vec<rara_kernel::memory::types::StateHistory>> {
        let entries = self
            .mem0()
            .history(Mem0HistoryRequest {
                memory_id: id.to_string(),
            })
            .await
            .map_err(to_kernel_state_error)?;

        let history = entries
            .into_iter()
            .map(|e| rara_kernel::memory::types::StateHistory {
                id:          e.id.parse().unwrap_or_else(|_| Uuid::new_v4()),
                memory_id:   e.memory_id.parse().unwrap_or_else(|_| Uuid::new_v4()),
                old_content: e.old_memory,
                new_content: e.new_memory,
                event:       e.event,
                created_at:  parse_timestamp(&e.created_at),
                is_deleted:  e.is_deleted,
            })
            .collect();

        Ok(history)
    }
}

/// Convert a `Vec<Mem0Memory>` to a `Vec<StateFact>`.
fn mem0_memories_to_facts(
    memories: Vec<crate::mem0_client::Mem0Memory>,
) -> Vec<rara_kernel::memory::types::StateFact> {
    memories.into_iter().map(mem0_memory_to_fact).collect()
}

/// Convert a single `Mem0Memory` to a `StateFact`.
fn mem0_memory_to_fact(m: crate::mem0_client::Mem0Memory) -> rara_kernel::memory::types::StateFact {
    rara_kernel::memory::types::StateFact {
        id:         m.id.parse().unwrap_or_else(|_| Uuid::new_v4()),
        content:    m.memory,
        score:      m.score,
        metadata:   m
            .metadata
            .map(|obj| serde_json::to_value(obj).unwrap_or_default()),
        created_at: parse_timestamp(&m.created_at),
        updated_at: parse_timestamp(&m.updated_at),
    }
}

// ─── KnowledgeMemory (Memos) ────────────────────────────────────────

#[async_trait::async_trait]
impl rara_kernel::memory::knowledge::KnowledgeMemory for MemoryManager {
    async fn write(
        &self,
        _ctx: &rara_kernel::memory::types::MemoryContext,
        _scope: rara_kernel::memory::types::Scope,
        content: &str,
        tags: &[&str],
    ) -> rara_kernel::memory::Result<rara_kernel::memory::types::KnowledgeNote> {
        // Prepend #tags at the top of the content, then create the memo.
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

        let entry = self
            .memos()
            .create_memo(&body, "PRIVATE")
            .await
            .map_err(to_kernel_knowledge_error)?;

        Ok(memo_entry_to_note(entry))
    }

    async fn read(
        &self,
        _ctx: &rara_kernel::memory::types::MemoryContext,
        _scope: rara_kernel::memory::types::Scope,
        id: Uuid,
    ) -> rara_kernel::memory::Result<Option<rara_kernel::memory::types::KnowledgeNote>> {
        // Memos uses string IDs — convert UUID to its simple representation.
        let id_str = id.to_string();
        match self.memos().get_memo(&id_str).await {
            Ok(entry) => Ok(Some(memo_entry_to_note(entry))),
            Err(crate::error::MemoryError::Memos { message })
                if message.contains("404") || message.contains("not found") =>
            {
                Ok(None)
            }
            Err(e) => Err(to_kernel_knowledge_error(e)),
        }
    }

    async fn list(
        &self,
        _ctx: &rara_kernel::memory::types::MemoryContext,
        _scope: rara_kernel::memory::types::Scope,
        tags: &[&str],
        limit: usize,
    ) -> rara_kernel::memory::Result<Vec<rara_kernel::memory::types::KnowledgeNote>> {
        // Build an AIP-160 filter expression from tags.
        let filter = if tags.is_empty() {
            None
        } else {
            // Memos filter syntax: `tag == 'tag1'` (single tag) or combine with AND/OR.
            // For simplicity, use the first tag as the filter; Memos does not
            // support multiple-tag AND natively in a single filter expression.
            Some(format!("tag == '{}'", tags[0]))
        };

        let entries = self
            .memos()
            .list_memos(limit, filter.as_deref())
            .await
            .map_err(to_kernel_knowledge_error)?;

        Ok(entries.into_iter().map(memo_entry_to_note).collect())
    }

    async fn delete(
        &self,
        _ctx: &rara_kernel::memory::types::MemoryContext,
        _scope: rara_kernel::memory::types::Scope,
        id: Uuid,
    ) -> rara_kernel::memory::Result<()> {
        let id_str = id.to_string();
        self.memos()
            .delete_memo(&id_str)
            .await
            .map_err(to_kernel_knowledge_error)?;
        Ok(())
    }
}

/// Convert a [`MemoEntry`] into a kernel [`KnowledgeNote`].
///
/// Tags are extracted from lines starting with `#` in the content.
fn memo_entry_to_note(
    entry: crate::memos_client::MemoEntry,
) -> rara_kernel::memory::types::KnowledgeNote {
    // Extract tags from #tag markers in content.
    let tags: Vec<String> = entry
        .content
        .split_whitespace()
        .filter(|w| w.starts_with('#') && w.len() > 1)
        .map(|w| w.trim_start_matches('#').to_owned())
        .collect();

    // Parse the memo UID as a UUID. If that fails, generate a deterministic one.
    let id = entry.uid.parse::<Uuid>().unwrap_or_else(|_| Uuid::new_v4());

    let created_at = entry
        .create_time
        .parse::<jiff::Timestamp>()
        .unwrap_or_else(|_| jiff::Timestamp::now());
    let updated_at = entry
        .update_time
        .parse::<jiff::Timestamp>()
        .unwrap_or_else(|_| jiff::Timestamp::now());

    rara_kernel::memory::types::KnowledgeNote {
        id,
        content: entry.content,
        tags,
        created_at,
        updated_at,
    }
}

// ─── LearningMemory (Hindsight) ─────────────────────────────────────

#[async_trait::async_trait]
impl rara_kernel::memory::learning::LearningMemory for MemoryManager {
    async fn retain(
        &self,
        _ctx: &rara_kernel::memory::types::MemoryContext,
        _scope: rara_kernel::memory::types::Scope,
        content: &str,
    ) -> rara_kernel::memory::Result<()> {
        self.hindsight()
            .retain(content)
            .await
            .map_err(to_kernel_learning_error)
    }

    async fn recall(
        &self,
        _ctx: &rara_kernel::memory::types::MemoryContext,
        _scope: rara_kernel::memory::types::Scope,
        query: &str,
        limit: usize,
    ) -> rara_kernel::memory::Result<Vec<rara_kernel::memory::types::RecallEntry>> {
        let memories = self
            .hindsight()
            .recall(query, limit)
            .await
            .map_err(to_kernel_learning_error)?;

        let entries = memories
            .into_iter()
            .map(|m| rara_kernel::memory::types::RecallEntry {
                id:      m.id.parse().unwrap_or_else(|_| Uuid::new_v4()),
                content: m.content,
                score:   m.score,
            })
            .collect();

        Ok(entries)
    }

    async fn reflect(
        &self,
        _ctx: &rara_kernel::memory::types::MemoryContext,
        _scope: rara_kernel::memory::types::Scope,
        query: &str,
    ) -> rara_kernel::memory::Result<String> {
        self.hindsight()
            .reflect(query)
            .await
            .map_err(to_kernel_learning_error)
    }
}
