use std::collections::HashMap;

use async_trait::async_trait;
use jiff::Timestamp;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::{
    channel::types::ChatMessage,
    session::{Exchange, SessionMeta, SessionStore},
};

/// In-memory session store for development and testing.
pub struct InMemorySessionStore {
    sessions: RwLock<HashMap<Uuid, SessionData>>,
}

struct SessionData {
    meta:     SessionMeta,
    messages: Vec<ChatMessage>,
}

impl InMemorySessionStore {
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
        }
    }
}

impl Default for InMemorySessionStore {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl SessionStore for InMemorySessionStore {
    async fn load_history(&self, session_id: Uuid) -> crate::error::Result<Vec<ChatMessage>> {
        let sessions = self.sessions.read().await;
        Ok(sessions
            .get(&session_id)
            .map(|s| s.messages.clone())
            .unwrap_or_default())
    }

    async fn append(&self, session_id: Uuid, exchange: Exchange) -> crate::error::Result<()> {
        let mut sessions = self.sessions.write().await;
        let data = sessions.entry(session_id).or_insert_with(|| {
            let now = Timestamp::now();
            SessionData {
                meta: SessionMeta {
                    id:         session_id,
                    title:      None,
                    created_at: now,
                    updated_at: now,
                },
                messages: Vec::new(),
            }
        });
        data.messages.push(exchange.user_message);
        data.messages.push(exchange.assistant_message);
        data.meta.updated_at = Timestamp::now();
        Ok(())
    }

    async fn get_or_create(&self, session_id: Uuid) -> crate::error::Result<SessionMeta> {
        let mut sessions = self.sessions.write().await;
        let data = sessions.entry(session_id).or_insert_with(|| {
            let now = Timestamp::now();
            SessionData {
                meta: SessionMeta {
                    id:         session_id,
                    title:      None,
                    created_at: now,
                    updated_at: now,
                },
                messages: Vec::new(),
            }
        });
        Ok(data.meta.clone())
    }

    async fn compact(&self, session_id: Uuid, summary: String) -> crate::error::Result<()> {
        let mut sessions = self.sessions.write().await;
        if let Some(data) = sessions.get_mut(&session_id) {
            data.messages.clear();
            data.messages
                .push(ChatMessage::system(format!("[Compacted summary] {summary}")));
            data.meta.updated_at = Timestamp::now();
        }
        Ok(())
    }
}
