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
use reqwest::{Client, StatusCode};
use serde::Deserialize;

use super::client::{
    BotServiceClient, BotServiceError, ChannelBinding, DiscoveryJob, McpServerInfo, SessionDetail,
    SessionListItem,
};

/// A [`BotServiceClient`] that calls [`SessionIndex`] and [`TapeService`]
/// directly, bypassing any HTTP layer.
pub struct KernelBotServiceClient {
    sessions: Arc<dyn SessionIndex>,
    tape: TapeService,
    gateway_base_url: Option<String>,
    http_client: Client,
}

impl KernelBotServiceClient {
    pub fn new(
        sessions: Arc<dyn SessionIndex>,
        tape: TapeService,
        gateway_base_url: Option<String>,
    ) -> Self {
        Self {
            sessions,
            tape,
            gateway_base_url,
            http_client: Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .expect("reqwest client should build"),
        }
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

    async fn restart_agent(&self) -> Result<(), BotServiceError> {
        self.post_gateway_action(
            "/gateway/restart",
            "restart",
            std::time::Duration::from_secs(5),
        )
        .await
    }

    async fn update_agent(&self) -> Result<String, BotServiceError> {
        let response: GatewayUpdateResponse = self
            .post_gateway_json(
                "/gateway/update",
                "update",
                std::time::Duration::from_secs(900),
            )
            .await?;
        Ok(response.message)
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

#[derive(Debug, Deserialize)]
struct GatewayUpdateResponse {
    message: String,
}

#[derive(Debug, Deserialize)]
struct GatewayErrorResponse {
    message: String,
}

impl KernelBotServiceClient {
    async fn post_gateway_action(
        &self,
        path: &str,
        action: &str,
        timeout: std::time::Duration,
    ) -> Result<(), BotServiceError> {
        let response = self.post_gateway(path, action, timeout).await?;
        if response.status() == StatusCode::OK {
            return Ok(());
        }

        Err(BotServiceError::Service {
            message: format!(
                "gateway {action} request returned HTTP {}",
                response.status()
            ),
        })
    }

    async fn post_gateway_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        action: &str,
        timeout: std::time::Duration,
    ) -> Result<T, BotServiceError> {
        let response = self.post_gateway(path, action, timeout).await?;
        let status = response.status();
        if !status.is_success() {
            let body = response
                .bytes()
                .await
                .map_err(|e| BotServiceError::Service {
                    message: format!("gateway {action} error response read failed: {e}"),
                })?;
            let message = serde_json::from_slice::<GatewayErrorResponse>(&body)
                .map(|payload| payload.message)
                .unwrap_or_else(|_| String::from_utf8_lossy(&body).trim().to_owned());
            return Err(BotServiceError::Service {
                message: format!("gateway {action} request returned HTTP {status}: {message}"),
            });
        }

        response
            .json::<T>()
            .await
            .map_err(|e| BotServiceError::Service {
                message: format!("gateway {action} response decode failed: {e}"),
            })
    }

    async fn post_gateway(
        &self,
        path: &str,
        action: &str,
        timeout: std::time::Duration,
    ) -> Result<reqwest::Response, BotServiceError> {
        let base_url = gateway_base_url(self.gateway_base_url.as_deref())?;
        let url = format!("{}{path}", normalize_gateway_base_url(base_url));
        self.http_client
            .post(url)
            .timeout(timeout)
            .send()
            .await
            .map_err(|e| BotServiceError::Service {
                message: format!("gateway {action} request failed: {e}"),
            })
    }
}

fn gateway_base_url(base_url: Option<&str>) -> Result<&str, BotServiceError> {
    base_url.ok_or_else(|| BotServiceError::Service {
        message: "gateway admin API is not configured".to_owned(),
    })
}

fn normalize_gateway_base_url(address: &str) -> String {
    if address.starts_with("http://") || address.starts_with("https://") {
        address.trim_end_matches('/').to_owned()
    } else {
        format!("http://{}", address.trim_end_matches('/'))
    }
}
