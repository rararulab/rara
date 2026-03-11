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

//! [`BotServiceClient`] implementation backed by kernel subsystems
//! ([`SessionIndex`] + [`TapeService`]).

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use rara_kernel::{
    memory::{TapEntryKind, TapeService, get_fork_metadata, set_fork_metadata},
    session::{self as ks, SessionIndex, SessionKey},
};

use super::client::{
    BotServiceClient, BotServiceError, ChannelBinding, DiscoveryJob, McpServerInfo, SessionDetail,
    SessionListItem,
};

/// A [`BotServiceClient`] that calls [`SessionIndex`] and [`TapeService`]
/// directly, bypassing any HTTP layer.
pub struct KernelBotServiceClient {
    sessions: Arc<dyn SessionIndex>,
    tape:     TapeService,
}

impl KernelBotServiceClient {
    pub fn new(sessions: Arc<dyn SessionIndex>, tape: TapeService) -> Self {
        Self { sessions, tape }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn map_session_err(e: ks::SessionError) -> BotServiceError {
    BotServiceError::Service {
        message: e.to_string(),
    }
}

fn map_tape_err(context: &'static str, e: rara_kernel::memory::TapError) -> BotServiceError {
    BotServiceError::Service {
        message: format!("{context}: {e}"),
    }
}

fn entry_to_list_item(e: &ks::SessionEntry) -> SessionListItem {
    SessionListItem {
        key:           e.key.to_string(),
        title:         e.title.clone(),
        message_count: e.message_count,
        updated_at:    e.updated_at.to_rfc3339(),
    }
}

fn entry_to_detail(e: &ks::SessionEntry) -> SessionDetail {
    SessionDetail {
        key:           e.key.to_string(),
        title:         e.title.clone(),
        model:         e.model.clone(),
        message_count: e.message_count,
        preview:       e.preview.clone(),
        created_at:    e.created_at.to_rfc3339(),
        updated_at:    e.updated_at.to_rfc3339(),
    }
}

fn binding_to_client(b: &ks::ChannelBinding) -> ChannelBinding {
    ChannelBinding {
        session_key: b.session_key.to_string(),
    }
}

// ---------------------------------------------------------------------------
// BotServiceClient impl
// ---------------------------------------------------------------------------

#[async_trait]
impl BotServiceClient for KernelBotServiceClient {
    // -- Session management --------------------------------------------------

    async fn get_channel_session(
        &self,
        chat_id: &str,
    ) -> Result<Option<ChannelBinding>, BotServiceError> {
        self.sessions
            .get_channel_binding("telegram", chat_id)
            .await
            .map(|opt| opt.as_ref().map(binding_to_client))
            .map_err(map_session_err)
    }

    async fn bind_channel(
        &self,
        channel_type: &str,
        chat_id: &str,
        session_key: &str,
    ) -> Result<ChannelBinding, BotServiceError> {
        let key = SessionKey::try_from_raw(session_key).map_err(|e| BotServiceError::Service {
            message: format!("invalid session key: {e}"),
        })?;
        let now = Utc::now();
        let binding = ks::ChannelBinding {
            channel_type: channel_type.to_owned(),
            chat_id:      chat_id.to_owned(),
            session_key:  key,
            created_at:   now,
            updated_at:   now,
        };
        self.sessions
            .bind_channel(&binding)
            .await
            .map(|b| binding_to_client(&b))
            .map_err(map_session_err)
    }

    async fn create_session(&self, title: Option<&str>) -> Result<String, BotServiceError> {
        let now = Utc::now();
        let entry = ks::SessionEntry {
            key:           SessionKey::new(),
            title:         title.map(String::from),
            model:         None,
            system_prompt: None,
            message_count: 0,
            preview:       None,
            metadata:      None,
            created_at:    now,
            updated_at:    now,
        };
        let created = self
            .sessions
            .create_session(&entry)
            .await
            .map_err(map_session_err)?;
        Ok(created.key.to_string())
    }

    async fn clear_session_messages(&self, session_key: &str) -> Result<(), BotServiceError> {
        // Reset (archive) the tape for this session.
        self.tape
            .reset(session_key, true)
            .await
            .map_err(|e| BotServiceError::Service {
                message: format!("failed to clear tape: {e}"),
            })?;
        Ok(())
    }

    async fn list_sessions(&self, limit: u32) -> Result<Vec<SessionListItem>, BotServiceError> {
        self.sessions
            .list_sessions(limit as i64, 0)
            .await
            .map(|v| v.iter().map(entry_to_list_item).collect())
            .map_err(map_session_err)
    }

    async fn get_session(&self, key: &str) -> Result<SessionDetail, BotServiceError> {
        let sk = SessionKey::try_from_raw(key).map_err(|e| BotServiceError::Service {
            message: format!("invalid session key: {e}"),
        })?;
        match self
            .sessions
            .get_session(&sk)
            .await
            .map_err(map_session_err)?
        {
            Some(entry) => Ok(entry_to_detail(&entry)),
            None => Err(BotServiceError::Service {
                message: format!("session not found: {key}"),
            }),
        }
    }

    async fn update_session(
        &self,
        key: &str,
        model: Option<&str>,
    ) -> Result<SessionDetail, BotServiceError> {
        let sk = SessionKey::try_from_raw(key).map_err(|e| BotServiceError::Service {
            message: format!("invalid session key: {e}"),
        })?;
        let mut entry = self
            .sessions
            .get_session(&sk)
            .await
            .map_err(map_session_err)?
            .ok_or_else(|| BotServiceError::Service {
                message: format!("session not found: {key}"),
            })?;
        if let Some(m) = model {
            entry.model = Some(m.to_owned());
        }
        let updated = self
            .sessions
            .update_session(&entry)
            .await
            .map_err(map_session_err)?;
        Ok(entry_to_detail(&updated))
    }

    async fn anchor_tree(
        &self,
        session_key: &str,
    ) -> Result<rara_kernel::memory::AnchorTree, BotServiceError> {
        self.tape
            .build_anchor_tree(session_key, &*self.sessions)
            .await
            .map_err(|e| map_tape_err("failed to build anchor tree", e))
    }

    async fn checkout_anchor(
        &self,
        session_key: &str,
        anchor_name: &str,
    ) -> Result<String, BotServiceError> {
        let entries = self
            .tape
            .entries(session_key)
            .await
            .map_err(|e| map_tape_err("failed to load tape", e))?;

        let anchor_entry_id = entries
            .iter()
            .rev()
            // Use the most recent anchor with this name, matching user
            // expectation when names repeat across handoffs.
            .find(|entry| {
                entry.kind == TapEntryKind::Anchor
                    && entry.payload.get("name").and_then(|v| v.as_str()) == Some(anchor_name)
            })
            .map(|entry| entry.id)
            .ok_or_else(|| BotServiceError::Service {
                message: format!("anchor not found: {anchor_name}"),
            })?;

        let fork_tape_name = self
            .tape
            .store()
            .fork(session_key, Some(anchor_entry_id))
            .await
            .map_err(|e| map_tape_err("failed to fork tape", e))?;

        let now = Utc::now();
        let new_key = SessionKey::new();
        let mut metadata = None;
        set_fork_metadata(&mut metadata, session_key, anchor_name);
        let entry = ks::SessionEntry {
            key: new_key.clone(),
            title: Some(format!("Fork from {anchor_name}")),
            model: None,
            system_prompt: None,
            message_count: 0,
            preview: None,
            metadata,
            created_at: now,
            updated_at: now,
        };

        let created = match self.sessions.create_session(&entry).await {
            Ok(created) => created,
            Err(e) => {
                let _ = self.tape.store().discard(&fork_tape_name).await;
                return Err(map_session_err(e));
            }
        };

        let new_tape = created.key.to_string();
        // Read full fork tape then re-append into the new session tape.
        // We intentionally avoid FileTapeStore::merge here because merge only
        // applies entries created *after* the fork point.
        let fork_entries = match self.tape.store().read(&fork_tape_name).await {
            Ok(Some(entries)) => entries,
            Ok(None) => Vec::new(),
            Err(e) => {
                let _ = self.tape.store().discard(&fork_tape_name).await;
                let _ = self.sessions.delete_session(&created.key).await;
                return Err(map_tape_err("failed to read forked tape", e));
            }
        };

        for entry in fork_entries {
            if let Err(e) = self
                .tape
                .store()
                .append(&new_tape, entry.kind, entry.payload, entry.metadata)
                .await
            {
                let _ = self.tape.store().discard(&fork_tape_name).await;
                let _ = self.tape.store().reset(&new_tape).await;
                let _ = self.sessions.delete_session(&created.key).await;
                return Err(map_tape_err("failed to copy forked entries", e));
            }
        }

        // Fork tape is temporary and should never remain after checkout.
        let _ = self.tape.store().discard(&fork_tape_name).await;
        Ok(created.key.to_string())
    }

    async fn parent_session(&self, session_key: &str) -> Result<Option<String>, BotServiceError> {
        let sk = SessionKey::try_from_raw(session_key).map_err(|e| BotServiceError::Service {
            message: format!("invalid session key: {e}"),
        })?;
        let entry = self
            .sessions
            .get_session(&sk)
            .await
            .map_err(map_session_err)?;
        Ok(entry.and_then(|session| {
            get_fork_metadata(&session.metadata).map(|metadata| metadata.forked_from)
        }))
    }

    // -- Job discovery (not yet implemented) ----------------------------------

    async fn discover_jobs(
        &self,
        _keywords: Vec<String>,
        _location: Option<String>,
        _max_results: u32,
    ) -> Result<Vec<DiscoveryJob>, BotServiceError> {
        Err(BotServiceError::Service {
            message: "job discovery not available via kernel client".to_owned(),
        })
    }

    async fn submit_jd_parse(&self, _text: &str) -> Result<(), BotServiceError> {
        Err(BotServiceError::Service {
            message: "JD parsing not available via kernel client".to_owned(),
        })
    }

    // -- MCP servers (not yet implemented) ------------------------------------

    async fn list_mcp_servers(&self) -> Result<Vec<McpServerInfo>, BotServiceError> {
        Err(BotServiceError::Service {
            message: "MCP management not available via kernel client".to_owned(),
        })
    }

    async fn get_mcp_server(&self, _name: &str) -> Result<McpServerInfo, BotServiceError> {
        Err(BotServiceError::Service {
            message: "MCP management not available via kernel client".to_owned(),
        })
    }

    async fn add_mcp_server(
        &self,
        _name: &str,
        _command: &str,
        _args: &[String],
    ) -> Result<McpServerInfo, BotServiceError> {
        Err(BotServiceError::Service {
            message: "MCP management not available via kernel client".to_owned(),
        })
    }

    async fn start_mcp_server(&self, _name: &str) -> Result<(), BotServiceError> {
        Err(BotServiceError::Service {
            message: "MCP management not available via kernel client".to_owned(),
        })
    }

    async fn remove_mcp_server(&self, _name: &str) -> Result<(), BotServiceError> {
        Err(BotServiceError::Service {
            message: "MCP management not available via kernel client".to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{path::Path, sync::Arc};

    use async_trait::async_trait;
    use chrono::Utc;
    use dashmap::DashMap;
    use rara_kernel::{
        memory::{HandoffState, TapEntryKind},
        session::{ChannelBinding, SessionEntry, SessionError, SessionIndex},
    };
    use serde_json::{Value, json};

    use super::*;

    #[derive(Default)]
    struct InMemorySessionIndex {
        sessions: DashMap<String, SessionEntry>,
        bindings: DashMap<(String, String), ChannelBinding>,
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

        async fn get_session(
            &self,
            key: &SessionKey,
        ) -> Result<Option<SessionEntry>, SessionError> {
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
            let mut sessions: Vec<SessionEntry> =
                self.sessions.iter().map(|entry| entry.clone()).collect();
            sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
            let start = offset.max(0) as usize;
            let take = limit.max(0) as usize;
            Ok(sessions.into_iter().skip(start).take(take).collect())
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

        async fn bind_channel(
            &self,
            binding: &ChannelBinding,
        ) -> Result<ChannelBinding, SessionError> {
            self.bindings.insert(
                (binding.channel_type.clone(), binding.chat_id.clone()),
                binding.clone(),
            );
            Ok(binding.clone())
        }

        async fn get_channel_binding(
            &self,
            channel_type: &str,
            chat_id: &str,
        ) -> Result<Option<ChannelBinding>, SessionError> {
            Ok(self
                .bindings
                .get(&(channel_type.to_owned(), chat_id.to_owned()))
                .map(|binding| binding.clone()))
        }
    }

    async fn temp_tape_service(dir: &Path) -> TapeService {
        let store = rara_kernel::memory::FileTapeStore::new(dir, dir)
            .await
            .unwrap();
        TapeService::new(store)
    }

    async fn create_session(index: &InMemorySessionIndex, key: &SessionKey) {
        let now = Utc::now();
        index
            .create_session(&SessionEntry {
                key:           key.clone(),
                title:         None,
                model:         None,
                system_prompt: None,
                message_count: 0,
                preview:       None,
                metadata:      None,
                created_at:    now,
                updated_at:    now,
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn checkout_anchor_copies_entries_and_writes_fork_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let tape = temp_tape_service(tmp.path()).await;
        let sessions = Arc::new(InMemorySessionIndex::default());
        let client = KernelBotServiceClient::new(sessions.clone(), tape.clone());

        let root_key = SessionKey::new();
        let root_raw = root_key.to_string();
        create_session(&sessions, &root_key).await;

        tape.ensure_bootstrap_anchor(&root_raw).await.unwrap();
        tape.handoff(&root_raw, "topic/a", HandoffState::default())
            .await
            .unwrap();
        tape.append_message(
            &root_raw,
            json!({"role":"user","content":"message after topic/a"}),
            None,
        )
        .await
        .unwrap();

        let new_key = client.checkout_anchor(&root_raw, "topic/a").await.unwrap();
        let entries = tape.entries(&new_key).await.unwrap();
        assert!(entries.iter().any(|entry| {
            entry.kind == TapEntryKind::Anchor
                && entry.payload.get("name").and_then(Value::as_str) == Some("topic/a")
        }));
        assert!(entries.iter().any(|entry| {
            entry.kind == TapEntryKind::Message
                && entry.payload.get("content").and_then(Value::as_str)
                    == Some("message after topic/a")
        }));

        let new_session = sessions
            .get_session(&SessionKey::try_from_raw(&new_key).unwrap())
            .await
            .unwrap()
            .unwrap();
        let metadata = get_fork_metadata(&new_session.metadata).unwrap();
        assert_eq!(metadata.forked_from, root_raw);
        assert_eq!(metadata.forked_at_anchor, "topic/a");
    }
}
