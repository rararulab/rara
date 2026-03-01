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
use chrono::Utc;
use serde_json::Value;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::{
    channel::types::{ChannelType, ChatMessage},
    event::{EventBus, EventFilter, EventStream, KernelEvent},
    guard::{Guard, GuardContext, Verdict},
    io::{
        bus::OutboxStore,
        ingress::{IdentityResolver, SessionResolver},
        types::{BusError, IngestError, MessageId, OutboundEnvelope},
    },
    memory::{
        Result as MemResult, knowledge::KnowledgeMemory, learning::LearningMemory,
        state::StateMemory, types::*,
    },
    model_repo::{ModelEntry, ModelRepo, ModelRepoError},
    process::{SessionId, principal::UserId},
    session::{
        ChannelBinding, SessionEntry, SessionError, SessionKey, SessionRepository,
    },
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
    async fn create_session(
        &self,
        entry: &SessionEntry,
    ) -> Result<SessionEntry, SessionError> {
        Ok(entry.clone())
    }

    async fn get_session(
        &self,
        _key: &SessionKey,
    ) -> Result<Option<SessionEntry>, SessionError> {
        Ok(None)
    }

    async fn list_sessions(
        &self,
        _limit: i64,
        _offset: i64,
    ) -> Result<Vec<SessionEntry>, SessionError> {
        Ok(vec![])
    }

    async fn update_session(
        &self,
        entry: &SessionEntry,
    ) -> Result<SessionEntry, SessionError> {
        Ok(entry.clone())
    }

    async fn delete_session(&self, _key: &SessionKey) -> Result<(), SessionError> {
        Ok(())
    }

    async fn append_message(
        &self,
        _session_key: &SessionKey,
        message: &ChatMessage,
    ) -> Result<ChatMessage, SessionError> {
        Ok(message.clone())
    }

    async fn read_messages(
        &self,
        _session_key: &SessionKey,
        _after_seq: Option<i64>,
        _limit: Option<i64>,
    ) -> Result<Vec<ChatMessage>, SessionError> {
        Ok(vec![])
    }

    async fn clear_messages(&self, _session_key: &SessionKey) -> Result<(), SessionError> {
        Ok(())
    }

    async fn fork_session(
        &self,
        _source_key: &SessionKey,
        target_key: &SessionKey,
        _fork_at_seq: i64,
    ) -> Result<SessionEntry, SessionError> {
        let now = Utc::now();
        Ok(SessionEntry {
            key:           target_key.clone(),
            title:         None,
            model:         None,
            system_prompt: None,
            message_count: 0,
            preview:       None,
            metadata:      None,
            created_at:    now,
            updated_at:    now,
        })
    }

    async fn bind_channel(
        &self,
        binding: &ChannelBinding,
    ) -> Result<ChannelBinding, SessionError> {
        Ok(binding.clone())
    }

    async fn get_channel_binding(
        &self,
        _channel_type: &str,
        _account: &str,
        _chat_id: &str,
    ) -> Result<Option<ChannelBinding>, SessionError> {
        Ok(None)
    }
}

// ---- NoopIdentityResolver ----

/// A no-op identity resolver for testing — maps to
/// `"{channel_type}:{platform_user_id}"`.
pub struct NoopIdentityResolver;

#[async_trait]
impl IdentityResolver for NoopIdentityResolver {
    async fn resolve(
        &self,
        channel_type: ChannelType,
        platform_user_id: &str,
        _platform_chat_id: Option<&str>,
    ) -> Result<UserId, IngestError> {
        Ok(UserId(format!("{}:{}", channel_type, platform_user_id)))
    }
}

// ---- NoopSessionResolver ----

/// A no-op session resolver for testing — maps to
/// `"{channel_type}:{platform_chat_id}"`.
pub struct NoopSessionResolver;

#[async_trait]
impl SessionResolver for NoopSessionResolver {
    async fn resolve(
        &self,
        _user: &UserId,
        channel_type: ChannelType,
        platform_chat_id: Option<&str>,
    ) -> Result<SessionId, IngestError> {
        let chat_id = platform_chat_id.unwrap_or("default");
        Ok(SessionId::new(format!("{}:{}", channel_type, chat_id)))
    }
}

// ---- NoopModelRepo ----

/// A no-op model repo for testing — always returns `None`.
pub struct NoopModelRepo;

#[async_trait]
impl ModelRepo for NoopModelRepo {
    async fn get(&self, _key: &str) -> Option<String> {
        None
    }

    async fn set(&self, _key: &str, _model: &str) -> Result<(), ModelRepoError> {
        Ok(())
    }

    async fn remove(&self, _key: &str) -> Result<(), ModelRepoError> {
        Ok(())
    }

    async fn list(&self) -> Vec<ModelEntry> {
        vec![]
    }

    async fn fallback_models(&self) -> Vec<String> {
        vec![]
    }

    async fn set_fallback_models(&self, _models: Vec<String>) -> Result<(), ModelRepoError> {
        Ok(())
    }
}
