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

//! Shared in-memory session index for kernel-internal tests.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use dashmap::DashMap;

use super::{ChannelBinding, SessionEntry, SessionError, SessionIndex, SessionKey};
use crate::channel::types::ChannelType;

/// A minimal in-memory [`SessionIndex`] for unit tests.
#[derive(Default)]
pub struct InMemorySessionIndex {
    pub sessions: DashMap<String, SessionEntry>,
    pub bindings: DashMap<(ChannelType, String, Option<String>), ChannelBinding>,
}

impl InMemorySessionIndex {
    pub fn new() -> Self { Self::default() }
}

#[async_trait]
impl SessionIndex for InMemorySessionIndex {
    async fn create_session(&self, entry: &SessionEntry) -> Result<SessionEntry, SessionError> {
        let key = entry.key.to_string();
        if self.sessions.contains_key(&key) {
            return Err(SessionError::AlreadyExists { key });
        }
        self.sessions.insert(key, entry.clone());
        Ok(entry.clone())
    }

    async fn get_session(&self, key: &SessionKey) -> Result<Option<SessionEntry>, SessionError> {
        Ok(self
            .sessions
            .get(&key.to_string())
            .map(|entry| entry.clone()))
    }

    async fn list_sessions(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<SessionEntry>, SessionError> {
        let mut entries: Vec<SessionEntry> =
            self.sessions.iter().map(|entry| entry.clone()).collect();
        entries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        let start = offset.max(0) as usize;
        let take = limit.max(0) as usize;
        Ok(entries.into_iter().skip(start).take(take).collect())
    }

    async fn update_session(&self, entry: &SessionEntry) -> Result<SessionEntry, SessionError> {
        let key = entry.key.to_string();
        if !self.sessions.contains_key(&key) {
            return Err(SessionError::NotFound { key });
        }
        self.sessions.insert(key, entry.clone());
        Ok(entry.clone())
    }

    async fn delete_session(&self, key: &SessionKey) -> Result<(), SessionError> {
        self.sessions.remove(&key.to_string());
        Ok(())
    }

    async fn bind_channel(&self, binding: &ChannelBinding) -> Result<ChannelBinding, SessionError> {
        self.bindings.insert(
            (
                binding.channel_type,
                binding.chat_id.clone(),
                binding.thread_id.clone(),
            ),
            binding.clone(),
        );
        Ok(binding.clone())
    }

    async fn get_channel_binding(
        &self,
        channel_type: ChannelType,
        chat_id: &str,
        thread_id: Option<&str>,
    ) -> Result<Option<ChannelBinding>, SessionError> {
        Ok(self
            .bindings
            .get(&(
                channel_type,
                chat_id.to_owned(),
                thread_id.map(|s| s.to_owned()),
            ))
            .map(|entry| entry.clone()))
    }

    async fn get_channel_binding_by_session(
        &self,
        key: &SessionKey,
    ) -> Result<Option<ChannelBinding>, SessionError> {
        let key_str = key.to_string();
        Ok(self
            .bindings
            .iter()
            .find(|entry| entry.value().session_key.to_string() == key_str)
            .map(|entry| entry.value().clone()))
    }

    async fn list_channel_bindings_by_session(
        &self,
        key: &SessionKey,
    ) -> Result<Vec<ChannelBinding>, SessionError> {
        let key_str = key.to_string();
        Ok(self
            .bindings
            .iter()
            .filter(|entry| entry.value().session_key.to_string() == key_str)
            .map(|entry| entry.value().clone())
            .collect())
    }

    async fn unbind_session(&self, key: &SessionKey) -> Result<(), SessionError> {
        let key_str = key.to_string();
        let to_remove: Vec<_> = self
            .bindings
            .iter()
            .filter(|entry| entry.value().session_key.to_string() == key_str)
            .map(|entry| entry.key().clone())
            .collect();
        for k in to_remove {
            self.bindings.remove(&k);
        }
        Ok(())
    }
}

/// Helper to create a test session with optional metadata.
pub async fn create_test_session(
    sessions: &Arc<InMemorySessionIndex>,
    key: &SessionKey,
    metadata: Option<serde_json::Value>,
) {
    let now = Utc::now();
    sessions
        .create_session(&SessionEntry {
            key: key.clone(),
            title: None,
            model: None,
            thinking_level: None,
            system_prompt: None,
            message_count: 0,
            preview: None,
            metadata,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
}
