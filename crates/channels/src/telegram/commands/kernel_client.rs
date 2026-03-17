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
    handle::KernelHandle,
    memory::{TapeService, get_fork_metadata, set_fork_metadata},
    session::{self as ks, SessionIndex, SessionKey},
};
use snafu::ResultExt;

use super::client::{
    BotServiceClient, BotServiceError, ChannelBinding, CheckoutResult, DiscoveryJob, McpServerInfo,
    SessionDetail, SessionListItem, SessionSnafu, TapeSnafu,
};

/// A [`BotServiceClient`] that calls [`SessionIndex`] and [`TapeService`]
/// directly, bypassing any HTTP layer.
pub struct KernelBotServiceClient {
    sessions: Arc<dyn SessionIndex>,
    tape:     TapeService,
    handle:   Option<KernelHandle>,
}

impl KernelBotServiceClient {
    /// Create a new client backed by kernel subsystems.
    pub fn new(
        sessions: Arc<dyn SessionIndex>,
        tape: TapeService,
        handle: impl Into<Option<KernelHandle>>,
    ) -> Self {
        Self {
            sessions,
            tape,
            handle: handle.into(),
        }
    }
}

fn entry_to_list_item(e: &ks::SessionEntry) -> SessionListItem {
    SessionListItem {
        key:           e.key.to_string(),
        title:         e.title.clone(),
        preview:       e.preview.clone(),
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
        channel_type: &str,
        chat_id: &str,
    ) -> Result<Option<ChannelBinding>, BotServiceError> {
        self.sessions
            .get_channel_binding(channel_type, chat_id)
            .await
            .map(|opt| opt.as_ref().map(binding_to_client))
            .context(SessionSnafu)
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
            .context(SessionSnafu)
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
            .context(SessionSnafu)?;
        Ok(created.key.to_string())
    }

    async fn clear_session_messages(&self, session_key: &str) -> Result<(), BotServiceError> {
        // Reset (archive) the tape for this session.
        self.tape
            .reset(session_key, true)
            .await
            .context(TapeSnafu {
                context: "failed to clear tape",
            })?;
        Ok(())
    }

    async fn list_sessions(&self, limit: u32) -> Result<Vec<SessionListItem>, BotServiceError> {
        self.sessions
            .list_sessions(limit as i64, 0)
            .await
            .map(|v| v.iter().map(entry_to_list_item).collect())
            .context(SessionSnafu)
    }

    async fn get_session(&self, key: &str) -> Result<SessionDetail, BotServiceError> {
        let sk = SessionKey::try_from_raw(key).map_err(|e| BotServiceError::Service {
            message: format!("invalid session key: {e}"),
        })?;
        match self.sessions.get_session(&sk).await.context(SessionSnafu)? {
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
            .context(SessionSnafu)?
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
            .context(SessionSnafu)?;
        Ok(entry_to_detail(&updated))
    }

    async fn anchor_tree(
        &self,
        session_key: &str,
    ) -> Result<rara_kernel::memory::AnchorTree, BotServiceError> {
        // Delegate full tree assembly to kernel memory service so Telegram
        // layer only consumes a render-ready structure.
        self.tape
            .build_anchor_tree(session_key, &*self.sessions)
            .await
            .context(TapeSnafu {
                context: "failed to build anchor tree",
            })
    }

    async fn checkout_anchor(
        &self,
        session_key: &str,
        anchor_name: &str,
    ) -> Result<String, BotServiceError> {
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
        let created = self
            .sessions
            .create_session(&entry)
            .await
            .context(SessionSnafu)?;

        if let Err(e) = self
            .tape
            .checkout_anchor(session_key, anchor_name, &new_key.to_string())
            .await
        {
            // Rollback: remove the session we just created so it doesn't dangle.
            let _ = self.sessions.delete_session(&created.key).await;
            return Err(e).context(TapeSnafu {
                context: "checkout anchor",
            });
        }

        Ok(new_key.to_string())
    }

    async fn parent_session(&self, session_key: &str) -> Result<Option<String>, BotServiceError> {
        // Parent relationship is modeled in session metadata (`forked_from`).
        let sk = SessionKey::try_from_raw(session_key).map_err(|e| BotServiceError::Service {
            message: format!("invalid session key: {e}"),
        })?;
        let entry = self.sessions.get_session(&sk).await.context(SessionSnafu)?;
        Ok(entry.and_then(|session| {
            get_fork_metadata(&session.metadata).map(|metadata| metadata.forked_from)
        }))
    }

    async fn checkout_session(
        &self,
        chat_id: &str,
        session_key: &str,
        anchor_name: Option<&str>,
    ) -> Result<CheckoutResult, BotServiceError> {
        // Treat empty anchor argument the same as omitted argument.
        let anchor_name = anchor_name.map(str::trim).filter(|name| !name.is_empty());

        match anchor_name {
            Some(anchor_name) => {
                // `/checkout <anchor>` => create child session then rebind chat.
                let child_key = self.checkout_anchor(session_key, anchor_name).await?;
                self.bind_channel("telegram", chat_id, &child_key).await?;
                Ok(CheckoutResult::ForkedFromAnchor {
                    anchor_name: anchor_name.to_owned(),
                    session_key: child_key,
                })
            }
            None => {
                // `/checkout` => move back to parent session if one exists.
                let Some(parent_key) = self.parent_session(session_key).await? else {
                    return Ok(CheckoutResult::NoParent);
                };
                self.bind_channel("telegram", chat_id, &parent_key).await?;
                Ok(CheckoutResult::SwitchedToParent {
                    session_key: parent_key,
                })
            }
        }
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

    async fn delete_session(&self, key: &str) -> Result<(), BotServiceError> {
        let sk = SessionKey::try_from_raw(key).map_err(|e| BotServiceError::Service {
            message: format!("invalid session key: {e}"),
        })?;
        // Stop any active turn before deleting data.
        //
        // The agent loop monitors `turn_cancel` at `tokio::select!` points.
        // After the turn task returns, the kernel's `handle_turn_completed`
        // transitions state from Active → Ready (for long-lived sessions).
        // We poll for that transition to guarantee no more tape writes can
        // occur before we delete the data.
        if let Some(ref handle) = self.handle {
            let pt = handle.process_table();
            let is_active = pt
                .with(&sk, |s| {
                    s.state == rara_kernel::session::SessionState::Active
                })
                .unwrap_or(false);

            if is_active {
                pt.cancel_turn(&sk);

                // Poll until the turn completes and state leaves Active.
                // Bounded at 50 × 100ms = 5s to avoid hanging forever.
                for _ in 0..50 {
                    let still_active = pt
                        .with(&sk, |s| {
                            s.state == rara_kernel::session::SessionState::Active
                        })
                        .unwrap_or(false);
                    if !still_active {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            }

            // Prevent new work and mark as suspended.
            if pt.contains(&sk) {
                pt.cancel_process(&sk);
                let _ = pt.set_state(sk.clone(), rara_kernel::session::SessionState::Suspended);
            }
        }
        // Delete tape (message history).
        self.tape.delete_tape(key).await.context(TapeSnafu {
            context: "failed to delete tape",
        })?;
        // Remove channel bindings pointing to this session.
        self.sessions
            .unbind_session(&sk)
            .await
            .context(SessionSnafu)?;
        // Delete session metadata.
        self.sessions
            .delete_session(&sk)
            .await
            .context(SessionSnafu)?;
        Ok(())
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

    // NOTE: duplicated from rara_kernel::session::test_utils — extract to shared
    // test crate if more duplications appear.
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
        let client = KernelBotServiceClient::new(sessions.clone(), tape.clone(), None);

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
        // The message appended *after* the anchor should NOT be in the fork.
        assert!(!entries.iter().any(|entry| {
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
