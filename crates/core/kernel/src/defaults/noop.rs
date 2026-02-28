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

//! Noop implementations of kernel components for testing.

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::{
    channel::types::ChatMessage,
    event::{EventBus, EventFilter, EventStream, KernelEvent},
    guard::{Guard, GuardContext, Verdict},
    io::{
        bus::OutboxStore,
        types::{BusError, MessageId, OutboundEnvelope},
    },
    memory::{
        Result as MemResult, knowledge::KnowledgeMemory, learning::LearningMemory,
        state::StateMemory, types::*,
    },
    process::{SessionId, principal::UserId},
    session_manager::{SessionManagerError, SessionRepository},
};

// ---- NoopGuard ----

/// A guard that allows everything — no approval or moderation.
pub struct NoopGuard;

#[async_trait]
impl Guard for NoopGuard {
    async fn check_tool(&self, _ctx: &GuardContext, _tool_name: &str, _args: &Value) -> Verdict {
        Verdict::Allow
    }

    async fn check_output(&self, _ctx: &GuardContext, _content: &str) -> Verdict { Verdict::Allow }
}

// ---- NoopEventBus ----

/// An event bus that silently discards all published events.
pub struct NoopEventBus;

#[async_trait]
impl EventBus for NoopEventBus {
    async fn publish(&self, _event: KernelEvent) {
        // discard
    }

    async fn subscribe(&self, _filter: EventFilter) -> EventStream {
        let (_tx, rx) = broadcast::channel(1);
        rx
    }
}

// ---- NoopMemory ----

/// A memory implementation that does nothing — all ops succeed with empty
/// results.
pub struct NoopMemory;

#[async_trait]
impl StateMemory for NoopMemory {
    async fn add(
        &self,
        _ctx: &MemoryContext,
        _scope: Scope,
        _messages: Vec<Message>,
    ) -> MemResult<Vec<StateEvent>> {
        Ok(vec![])
    }

    async fn search(
        &self,
        _ctx: &MemoryContext,
        _scope: Scope,
        _query: &str,
        _limit: usize,
    ) -> MemResult<Vec<StateFact>> {
        Ok(vec![])
    }

    async fn get(
        &self,
        _ctx: &MemoryContext,
        _scope: Scope,
        _id: Uuid,
    ) -> MemResult<Option<StateFact>> {
        Ok(None)
    }

    async fn get_all(
        &self,
        _ctx: &MemoryContext,
        _scope: Scope,
        _limit: usize,
    ) -> MemResult<Vec<StateFact>> {
        Ok(vec![])
    }

    async fn update(
        &self,
        _ctx: &MemoryContext,
        _scope: Scope,
        _id: Uuid,
        _data: &str,
    ) -> MemResult<()> {
        Ok(())
    }

    async fn delete(&self, _ctx: &MemoryContext, _scope: Scope, _id: Uuid) -> MemResult<()> {
        Ok(())
    }

    async fn delete_all(&self, _ctx: &MemoryContext, _scope: Scope) -> MemResult<()> { Ok(()) }

    async fn history(
        &self,
        _ctx: &MemoryContext,
        _scope: Scope,
        _id: Uuid,
    ) -> MemResult<Vec<StateHistory>> {
        Ok(vec![])
    }
}

#[async_trait]
impl KnowledgeMemory for NoopMemory {
    async fn write(
        &self,
        _ctx: &MemoryContext,
        _scope: Scope,
        _content: &str,
        _tags: &[&str],
    ) -> MemResult<KnowledgeNote> {
        Ok(KnowledgeNote {
            id:         Uuid::new_v4(),
            content:    String::new(),
            tags:       vec![],
            created_at: jiff::Timestamp::now(),
            updated_at: jiff::Timestamp::now(),
        })
    }

    async fn read(
        &self,
        _ctx: &MemoryContext,
        _scope: Scope,
        _id: Uuid,
    ) -> MemResult<Option<KnowledgeNote>> {
        Ok(None)
    }

    async fn list(
        &self,
        _ctx: &MemoryContext,
        _scope: Scope,
        _tags: &[&str],
        _limit: usize,
    ) -> MemResult<Vec<KnowledgeNote>> {
        Ok(vec![])
    }

    async fn delete(&self, _ctx: &MemoryContext, _scope: Scope, _id: Uuid) -> MemResult<()> {
        Ok(())
    }
}

#[async_trait]
impl LearningMemory for NoopMemory {
    async fn retain(&self, _ctx: &MemoryContext, _scope: Scope, _content: &str) -> MemResult<()> {
        Ok(())
    }

    async fn recall(
        &self,
        _ctx: &MemoryContext,
        _scope: Scope,
        _query: &str,
        _limit: usize,
    ) -> MemResult<Vec<RecallEntry>> {
        Ok(vec![])
    }

    async fn reflect(
        &self,
        _ctx: &MemoryContext,
        _scope: Scope,
        _query: &str,
    ) -> MemResult<String> {
        Ok(String::new())
    }
}

// ---- NoopOutboxStore ----

/// A no-op outbox store for testing — all operations succeed without
/// persisting.
pub struct NoopOutboxStore;

#[async_trait]
impl OutboxStore for NoopOutboxStore {
    async fn append(&self, _envelope: OutboundEnvelope) -> Result<(), BusError> { Ok(()) }

    async fn drain_pending(&self, _max: usize) -> Vec<OutboundEnvelope> { vec![] }

    async fn mark_delivered(&self, _id: &MessageId) -> Result<(), BusError> { Ok(()) }
}

// ---- NoopSessionRepository ----

/// A no-op session repository for testing — all operations succeed without
/// persisting.
pub struct NoopSessionRepository;

#[async_trait]
impl SessionRepository for NoopSessionRepository {
    async fn ensure_session(
        &self,
        _id: &SessionId,
        _user: &UserId,
    ) -> Result<(), SessionManagerError> {
        Ok(())
    }

    async fn get_history(&self, _id: &SessionId) -> Result<Vec<ChatMessage>, SessionManagerError> {
        Ok(vec![])
    }

    async fn append_user_message(
        &self,
        _id: &SessionId,
        _content: &str,
    ) -> Result<(), SessionManagerError> {
        Ok(())
    }

    async fn append_assistant_message(
        &self,
        _id: &SessionId,
        _content: &str,
    ) -> Result<(), SessionManagerError> {
        Ok(())
    }
}
