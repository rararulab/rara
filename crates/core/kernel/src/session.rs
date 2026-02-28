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

//! Session store abstraction — conversation history persistence.

use async_trait::async_trait;
use jiff::Timestamp;
use uuid::Uuid;

use crate::channel::types::ChatMessage;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Metadata for a conversation session.
#[derive(Debug, Clone)]
pub struct SessionMeta {
    pub id:         Uuid,
    pub title:      Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// A single exchange (user message + assistant response).
#[derive(Debug, Clone)]
pub struct Exchange {
    pub user_message:      ChatMessage,
    pub assistant_message: ChatMessage,
}

// ---------------------------------------------------------------------------
// SessionStore trait
// ---------------------------------------------------------------------------

/// Conversation history persistence and management.
#[async_trait]
pub trait SessionStore: Send + Sync {
    /// Load all messages for a session.
    async fn load_history(&self, session_id: Uuid) -> crate::error::Result<Vec<ChatMessage>>;

    /// Append one exchange (user + assistant) to a session.
    async fn append(&self, session_id: Uuid, exchange: Exchange) -> crate::error::Result<()>;

    /// Get or create session metadata.
    async fn get_or_create(&self, session_id: Uuid) -> crate::error::Result<SessionMeta>;

    /// Compact history by replacing old messages with a summary.
    async fn compact(&self, session_id: Uuid, summary: String) -> crate::error::Result<()>;
}
