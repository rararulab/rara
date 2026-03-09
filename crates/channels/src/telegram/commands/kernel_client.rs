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
    memory::TapeService,
    session::{self as ks, SessionIndex, SessionKey},
};
use reqwest::{Client, Method};
use serde::de::DeserializeOwned;

use super::client::{
    BotServiceClient, BotServiceError, ChannelBinding, DiscoveryJob, GatewayCommandOutcome,
    GatewayStatus, McpServerInfo, SessionDetail, SessionListItem,
};

/// A [`BotServiceClient`] that calls [`SessionIndex`] and [`TapeService`]
/// directly, bypassing any HTTP layer.
pub struct KernelBotServiceClient {
    sessions: Arc<dyn SessionIndex>,
    tape: TapeService,
    gateway_admin: Option<GatewayAdminClient>,
}

impl KernelBotServiceClient {
    pub fn new(
        sessions: Arc<dyn SessionIndex>,
        tape: TapeService,
        gateway_admin: Option<GatewayAdminClient>,
    ) -> Self {
        Self {
            sessions,
            tape,
            gateway_admin,
        }
    }
}

#[derive(Clone)]
pub struct GatewayAdminClient {
    client: Client,
    base_url: String,
    owner_token: String,
}

impl GatewayAdminClient {
    pub fn new(base_url: impl Into<String>, owner_token: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            owner_token: owner_token.into(),
        }
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, BotServiceError> {
        self.request(Method::GET, path).await
    }

    async fn post<T: DeserializeOwned>(&self, path: &str) -> Result<T, BotServiceError> {
        self.request(Method::POST, path).await
    }

    async fn request<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
    ) -> Result<T, BotServiceError> {
        let url = format!("{}{}", self.base_url, path);
        let response = self
            .client
            .request(method, &url)
            .bearer_auth(&self.owner_token)
            .send()
            .await
            .map_err(|e| BotServiceError::Service {
                message: format!("gateway request failed: {e}"),
            })?;

        let status = response.status();
        if !status.is_success() {
            let detail = response.text().await.unwrap_or_else(|_| String::new());
            return Err(BotServiceError::Service {
                message: format!("gateway request failed with {}: {}", status, detail.trim()),
            });
        }

        response
            .json::<T>()
            .await
            .map_err(|e| BotServiceError::Service {
                message: format!("gateway response decode failed: {e}"),
            })
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

fn entry_to_list_item(e: &ks::SessionEntry) -> SessionListItem {
    SessionListItem {
        key: e.key.to_string(),
        title: e.title.clone(),
        message_count: e.message_count,
        updated_at: e.updated_at.to_rfc3339(),
    }
}

fn entry_to_detail(e: &ks::SessionEntry) -> SessionDetail {
    SessionDetail {
        key: e.key.to_string(),
        title: e.title.clone(),
        model: e.model.clone(),
        message_count: e.message_count,
        preview: e.preview.clone(),
        created_at: e.created_at.to_rfc3339(),
        updated_at: e.updated_at.to_rfc3339(),
    }
}

fn binding_to_client(b: &ks::ChannelBinding) -> ChannelBinding {
    ChannelBinding {
        session_key: b.session_key.to_string(),
    }
}

fn gateway_client(
    client: &Option<GatewayAdminClient>,
) -> Result<&GatewayAdminClient, BotServiceError> {
    client.as_ref().ok_or_else(|| BotServiceError::Service {
        message: "gateway operations are unavailable: gateway.bind_address or owner_token is not configured".to_owned(),
    })
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
            chat_id: chat_id.to_owned(),
            session_key: key,
            created_at: now,
            updated_at: now,
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
            key: SessionKey::new(),
            title: title.map(String::from),
            model: None,
            system_prompt: None,
            message_count: 0,
            preview: None,
            metadata: None,
            created_at: now,
            updated_at: now,
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

    async fn gateway_status(&self) -> Result<GatewayStatus, BotServiceError> {
        gateway_client(&self.gateway_admin)?
            .get("/gateway/status")
            .await
    }

    async fn gateway_restart(&self) -> Result<GatewayCommandOutcome, BotServiceError> {
        gateway_client(&self.gateway_admin)?
            .post("/gateway/restart")
            .await
    }

    async fn gateway_update(&self) -> Result<GatewayCommandOutcome, BotServiceError> {
        gateway_client(&self.gateway_admin)?
            .post("/gateway/update")
            .await
    }
}
