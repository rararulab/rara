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

//! Typed HTTP client from the bot process to the main service.
//!
//! This client uses locally duplicated request/response models
//! ([`DiscoveryCriteria`], [`DiscoveryJobResponse`]) so the bot crate
//! doesn't depend on the domain-job crate (same pattern as
//! [`ChatStreamEvent`]).
//!
//! # Endpoints
//!
//! | Method | Path                             | Purpose                          |
//! |--------|----------------------------------|----------------------------------|
//! | POST   | `/api/v1/jobs/discover`          | Search jobs with keyword filters |
//! | POST   | `/api/v1/internal/bot/jd-parse`  | Submit raw JD text for parsing   |

use futures::StreamExt;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use snafu::{ResultExt, Snafu};
use tokio::sync::mpsc;

// Local mirrors of domain-job types -- duplicated so the bot crate doesn't
// depend on the domain-job crate.

/// Request body for `POST /api/v1/jobs/discover`.
#[derive(Debug, Serialize)]
struct DiscoveryCriteria {
    pub keywords:     Vec<String>,
    pub location:     Option<String>,
    pub job_type:     Option<String>,
    pub max_results:  Option<u32>,
    pub posted_after: Option<String>,
    #[serde(default)]
    pub sites:        Vec<String>,
}

/// Subset of discovery response fields used by the bot's formatting logic.
#[derive(Debug, Deserialize)]
pub struct DiscoveryJobResponse {
    pub title:            String,
    pub company:          String,
    pub location:         Option<String>,
    pub url:              Option<String>,
    pub salary_min:       Option<i32>,
    pub salary_max:       Option<i32>,
    pub salary_currency:  Option<String>,
}

/// Error model for bot -> main-service HTTP calls.
#[derive(Debug, Snafu)]
pub enum MainServiceHttpError {
    #[snafu(display("request failed: {source}"))]
    Request { source: reqwest::Error },
    #[snafu(display("main service returned status {status}: {body}"))]
    HttpStatus { status: StatusCode, body: String },
}

/// Main service HTTP client used by bot runtime.
#[derive(Clone)]
pub struct MainServiceHttpClient {
    base_url: String,
    client:   reqwest::Client,
}

impl MainServiceHttpClient {
    /// Create a client with normalized base URL.
    pub fn new(base_url: String) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            client:   reqwest::Client::new(),
        }
    }

    /// Call main service discovery API.
    ///
    /// Maps directly to `POST /api/v1/jobs/discover`.
    pub async fn discover_jobs(
        &self,
        keywords: Vec<String>,
        location: Option<String>,
        max_results: u32,
    ) -> Result<Vec<DiscoveryJobResponse>, MainServiceHttpError> {
        let url = format!("{}/api/v1/jobs/discover", self.base_url);
        let req = DiscoveryCriteria {
            keywords,
            location,
            job_type: None,
            max_results: Some(max_results),
            posted_after: None,
            sites: Vec::new(),
        };

        let resp = self
            .client
            .post(url)
            .json(&req)
            .send()
            .await
            .context(RequestSnafu)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(MainServiceHttpError::HttpStatus { status, body });
        }

        let jobs = resp
            .json::<Vec<DiscoveryJobResponse>>()
            .await
            .context(RequestSnafu)?;
        Ok(jobs)
    }

    /// Submit a raw JD text to main service for parse-and-save flow.
    ///
    /// Maps to bot internal endpoint:
    /// `POST /api/v1/internal/bot/jd-parse`.
    pub async fn submit_jd_parse(&self, text: &str) -> Result<(), MainServiceHttpError> {
        let url = format!("{}/api/v1/internal/bot/jd-parse", self.base_url);
        let resp = self
            .client
            .post(url)
            .json(&JdParseRequest {
                text: text.to_owned(),
            })
            .send()
            .await
            .context(RequestSnafu)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(MainServiceHttpError::HttpStatus { status, body });
        }

        Ok(())
    }

    // -- Chat API methods ----------------------------------------------------

    /// Resolve channel binding to find the associated session key.
    ///
    /// Maps to `GET /api/v1/chat/channel-bindings/{type}/{account}/{id}`.
    pub async fn get_channel_session(
        &self,
        account: &str,
        chat_id: &str,
    ) -> Result<Option<ChannelBindingResponse>, MainServiceHttpError> {
        let url = format!(
            "{}/api/v1/chat/channel-bindings/telegram/{}/{}",
            self.base_url, account, chat_id
        );
        let resp = self.client.get(url).send().await.context(RequestSnafu)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(MainServiceHttpError::HttpStatus { status, body });
        }

        let binding = resp
            .json::<Option<ChannelBindingResponse>>()
            .await
            .context(RequestSnafu)?;
        Ok(binding)
    }

    /// Create or update a channel binding.
    ///
    /// Maps to `PUT /api/v1/chat/channel-bindings`.
    pub async fn bind_channel(
        &self,
        channel_type: &str,
        account: &str,
        chat_id: &str,
        session_key: &str,
    ) -> Result<ChannelBindingResponse, MainServiceHttpError> {
        let url = format!("{}/api/v1/chat/channel-bindings", self.base_url);
        let resp = self
            .client
            .put(url)
            .json(&serde_json::json!({
                "channel_type": channel_type,
                "account": account,
                "chat_id": chat_id,
                "session_key": session_key,
            }))
            .send()
            .await
            .context(RequestSnafu)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(MainServiceHttpError::HttpStatus { status, body });
        }

        resp.json::<ChannelBindingResponse>()
            .await
            .context(RequestSnafu)
    }

    /// Send a message to a chat session and get the assistant's response.
    ///
    /// Maps to `POST /api/v1/chat/sessions/{key}/send`.
    ///
    /// This request may take a long time (30-60s) because it involves LLM
    /// inference. The method uses an extended timeout to accommodate this.
    ///
    /// When `image_urls` is non-empty, the request body includes an
    /// `image_urls` array so the chat service can forward them to the LLM
    /// as multimodal content (e.g. base64 data URLs).
    pub async fn send_chat_message(
        &self,
        session_key: &str,
        text: &str,
        image_urls: Vec<String>,
    ) -> Result<ChatMessageResponse, MainServiceHttpError> {
        let url = format!(
            "{}/api/v1/chat/sessions/{}/send",
            self.base_url, session_key
        );

        // Use a dedicated client with extended timeout for LLM calls.
        let llm_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        let mut body = serde_json::json!({ "text": text });
        if !image_urls.is_empty() {
            body["image_urls"] = serde_json::json!(image_urls);
        }

        let resp = llm_client
            .post(url)
            .json(&body)
            .send()
            .await
            .context(RequestSnafu)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(MainServiceHttpError::HttpStatus { status, body });
        }

        resp.json::<ChatMessageResponse>()
            .await
            .context(RequestSnafu)
    }

    /// Send a chat message via the SSE streaming endpoint.
    ///
    /// Maps to `POST /api/v1/chat/sessions/{key}/stream`.
    ///
    /// Unlike [`send_chat_message`] which blocks until the full response is
    /// ready, this method returns immediately after the HTTP connection is
    /// established. A background tokio task consumes the SSE byte stream,
    /// parses each event, and forwards it through the returned
    /// `mpsc::Receiver<ChatStreamEvent>`.
    ///
    /// # SSE wire format
    ///
    /// The server sends events in standard SSE format:
    /// ```text
    /// event: text_delta
    /// data: {"type":"text_delta","text":"Hello"}
    ///
    /// event: done
    /// data: {"type":"done","text":"Hello, world!"}
    /// ```
    ///
    /// Keep-alive comments (`:keep-alive\n\n`) are silently discarded.
    ///
    /// # Terminal events
    ///
    /// The stream ends after either a `Done` or `Error` event. The background
    /// task exits and the channel closes, causing `rx.recv()` to return `None`.
    ///
    /// # Errors
    ///
    /// Returns `Err` only if the initial HTTP request fails (network error or
    /// non-2xx status). Stream-level errors (malformed SSE, etc.) are logged
    /// but do not propagate — the channel simply closes.
    pub async fn send_chat_message_streaming(
        &self,
        session_key: &str,
        text: &str,
        image_urls: Vec<String>,
    ) -> Result<mpsc::Receiver<ChatStreamEvent>, MainServiceHttpError> {
        let url = format!(
            "{}/api/v1/chat/sessions/{}/stream",
            self.base_url, session_key
        );

        let mut body = serde_json::json!({ "text": text });
        if !image_urls.is_empty() {
            body["image_urls"] = serde_json::json!(image_urls);
        }

        let resp = self
            .client
            .post(url)
            .json(&body)
            .send()
            .await
            .context(RequestSnafu)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(MainServiceHttpError::HttpStatus { status, body });
        }

        let (tx, rx) = mpsc::channel(64);

        // Spawn a background task to consume the SSE byte stream.
        // The task reads raw bytes from the HTTP response, buffers them,
        // and splits on "\n\n" boundaries (the SSE message delimiter).
        tokio::spawn(async move {
            // `bytes_stream()` yields raw HTTP response body chunks.
            // reqwest's `stream` feature must be enabled for this.
            let mut stream = resp.bytes_stream();
            // Rolling buffer for incomplete SSE messages that span chunk boundaries.
            let mut buffer = String::new();
            // Tracks the most recent `event:` field (used for debug logging only).
            let mut current_event_type = String::new();

            while let Some(chunk_result) = stream.next().await {
                let Ok(bytes) = chunk_result else { break };
                buffer.push_str(&String::from_utf8_lossy(&bytes));

                // SSE messages are delimited by double newlines. There may be
                // multiple complete messages in a single TCP chunk, or a message
                // may span multiple chunks. We loop to drain all complete messages.
                while let Some(pos) = buffer.find("\n\n") {
                    let message = buffer[..pos].to_owned();
                    buffer = buffer[pos + 2..].to_owned();

                    // Parse SSE fields from the message block.
                    // Each line is either `event: <type>`, `data: <json>`, or
                    // a comment starting with `:` (used for keep-alive).
                    let mut data_str = String::new();
                    for line in message.lines() {
                        if let Some(evt) = line.strip_prefix("event: ") {
                            current_event_type = evt.trim().to_owned();
                        } else if let Some(d) = line.strip_prefix("data: ") {
                            data_str = d.to_owned();
                        } else if line.starts_with(':') {
                            // SSE comment (keep-alive heartbeat), skip.
                            continue;
                        }
                    }

                    // Skip events without data (e.g. pure keep-alive).
                    if data_str.is_empty() {
                        continue;
                    }

                    // Deserialize the JSON payload using the tagged enum format.
                    // The `type` field in the JSON determines the variant.
                    match serde_json::from_str::<ChatStreamEvent>(&data_str) {
                        Ok(event) => {
                            let is_terminal = matches!(
                                &event,
                                ChatStreamEvent::Done { .. }
                                    | ChatStreamEvent::Error { .. }
                            );
                            if tx.send(event).await.is_err() {
                                return; // receiver dropped (caller disconnected)
                            }
                            if is_terminal {
                                return; // stream is complete
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                event_type = %current_event_type,
                                data = %data_str,
                                error = %e,
                                "failed to parse SSE event"
                            );
                        }
                    }
                }
            }
        });

        Ok(rx)
    }

    /// Create a new chat session.
    ///
    /// Maps to `POST /api/v1/chat/sessions`.
    pub async fn create_session(
        &self,
        key: &str,
        title: Option<&str>,
    ) -> Result<(), MainServiceHttpError> {
        let url = format!("{}/api/v1/chat/sessions", self.base_url);
        let resp = self
            .client
            .post(url)
            .json(&serde_json::json!({
                "key": key,
                "title": title,
            }))
            .send()
            .await
            .context(RequestSnafu)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(MainServiceHttpError::HttpStatus { status, body });
        }

        Ok(())
    }

    /// Clear all messages in a session.
    ///
    /// Maps to `DELETE /api/v1/chat/sessions/{key}/messages`.
    pub async fn clear_session_messages(
        &self,
        session_key: &str,
    ) -> Result<(), MainServiceHttpError> {
        let url = format!(
            "{}/api/v1/chat/sessions/{}/messages",
            self.base_url, session_key
        );
        let resp = self.client.delete(url).send().await.context(RequestSnafu)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(MainServiceHttpError::HttpStatus { status, body });
        }

        Ok(())
    }

    /// List chat sessions.
    ///
    /// Maps to `GET /api/v1/chat/sessions?limit=N`.
    pub async fn list_sessions(
        &self,
        limit: u32,
    ) -> Result<Vec<SessionListItem>, MainServiceHttpError> {
        let url = format!("{}/api/v1/chat/sessions?limit={}", self.base_url, limit);
        let resp = self.client.get(url).send().await.context(RequestSnafu)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(MainServiceHttpError::HttpStatus { status, body });
        }

        resp.json::<Vec<SessionListItem>>()
            .await
            .context(RequestSnafu)
    }

    /// List MCP server status.
    ///
    /// Maps to `GET /api/v1/mcp/servers`.
    pub async fn list_mcp_servers(&self) -> Result<Vec<McpServerInfo>, MainServiceHttpError> {
        let url = format!("{}/api/v1/mcp/servers", self.base_url);
        let resp = self.client.get(url).send().await.context(RequestSnafu)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(MainServiceHttpError::HttpStatus { status, body });
        }

        resp.json::<Vec<McpServerInfo>>()
            .await
            .context(RequestSnafu)
    }

    /// Add an MCP server and auto-start it.
    ///
    /// Maps to `POST /api/v1/mcp/servers`.
    pub async fn add_mcp_server(
        &self,
        name: &str,
        command: &str,
        args: &[&str],
    ) -> Result<McpServerInfo, MainServiceHttpError> {
        let url = format!("{}/api/v1/mcp/servers", self.base_url);
        let resp = self
            .client
            .post(url)
            .json(&serde_json::json!({
                "name": name,
                "command": command,
                "args": args,
                "enabled": true,
            }))
            .send()
            .await
            .context(RequestSnafu)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(MainServiceHttpError::HttpStatus { status, body });
        }

        resp.json::<McpServerInfo>().await.context(RequestSnafu)
    }

    /// Start an existing MCP server.
    ///
    /// Maps to `POST /api/v1/mcp/servers/{name}/start`.
    pub async fn start_mcp_server(
        &self,
        name: &str,
    ) -> Result<(), MainServiceHttpError> {
        let url = format!("{}/api/v1/mcp/servers/{}/start", self.base_url, name);
        let resp = self.client.post(url).send().await.context(RequestSnafu)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(MainServiceHttpError::HttpStatus { status, body });
        }

        Ok(())
    }

    /// Remove an MCP server.
    ///
    /// Maps to `DELETE /api/v1/mcp/servers/{name}`.
    pub async fn remove_mcp_server(
        &self,
        name: &str,
    ) -> Result<(), MainServiceHttpError> {
        let url = format!("{}/api/v1/mcp/servers/{}", self.base_url, name);
        let resp = self.client.delete(url).send().await.context(RequestSnafu)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(MainServiceHttpError::HttpStatus { status, body });
        }

        Ok(())
    }

    /// Get a single MCP server's info.
    ///
    /// Maps to `GET /api/v1/mcp/servers/{name}`.
    pub async fn get_mcp_server(
        &self,
        name: &str,
    ) -> Result<McpServerInfo, MainServiceHttpError> {
        let url = format!("{}/api/v1/mcp/servers/{}", self.base_url, name);
        let resp = self.client.get(url).send().await.context(RequestSnafu)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(MainServiceHttpError::HttpStatus { status, body });
        }

        resp.json::<McpServerInfo>().await.context(RequestSnafu)
    }

    /// Update session fields (e.g. model, title).
    ///
    /// Maps to `PATCH /api/v1/chat/sessions/{key}`.
    pub async fn update_session(
        &self,
        key: &str,
        model: Option<&str>,
    ) -> Result<SessionDetailResponse, MainServiceHttpError> {
        let url = format!("{}/api/v1/chat/sessions/{}", self.base_url, key);
        let mut body = serde_json::Map::new();
        if let Some(m) = model {
            body.insert("model".to_owned(), serde_json::json!(m));
        }
        let resp = self
            .client
            .patch(url)
            .json(&body)
            .send()
            .await
            .context(RequestSnafu)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(MainServiceHttpError::HttpStatus { status, body });
        }

        resp.json::<SessionDetailResponse>()
            .await
            .context(RequestSnafu)
    }

    /// Get session details.
    ///
    /// Maps to `GET /api/v1/chat/sessions/{key}`.
    pub async fn get_session(
        &self,
        key: &str,
    ) -> Result<SessionDetailResponse, MainServiceHttpError> {
        let url = format!("{}/api/v1/chat/sessions/{}", self.base_url, key);
        let resp = self.client.get(url).send().await.context(RequestSnafu)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(MainServiceHttpError::HttpStatus { status, body });
        }

        resp.json::<SessionDetailResponse>()
            .await
            .context(RequestSnafu)
    }

    /// Dispatch a coding task.
    ///
    /// Maps to `POST /api/v1/coding-tasks`.
    pub async fn dispatch_coding_task(
        &self,
        prompt: &str,
        agent: &str,
    ) -> Result<CodingTaskResponse, MainServiceHttpError> {
        let url = format!("{}/api/v1/coding-tasks", self.base_url);
        let resp = self
            .client
            .post(url)
            .json(&serde_json::json!({
                "prompt": prompt,
                "agent_type": agent,
            }))
            .send()
            .await
            .context(RequestSnafu)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(MainServiceHttpError::HttpStatus { status, body });
        }

        resp.json::<CodingTaskResponse>()
            .await
            .context(RequestSnafu)
    }

    /// List coding tasks.
    ///
    /// Maps to `GET /api/v1/coding-tasks`.
    pub async fn list_coding_tasks(
        &self,
    ) -> Result<Vec<CodingTaskSummaryResponse>, MainServiceHttpError> {
        let url = format!("{}/api/v1/coding-tasks", self.base_url);
        let resp = self.client.get(url).send().await.context(RequestSnafu)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(MainServiceHttpError::HttpStatus { status, body });
        }

        resp.json::<Vec<CodingTaskSummaryResponse>>()
            .await
            .context(RequestSnafu)
    }
}

#[derive(Debug, Clone, Serialize)]
struct JdParseRequest {
    /// Raw JD text from telegram message.
    text: String,
}

// ---------------------------------------------------------------------------
// Chat API response types
// ---------------------------------------------------------------------------

/// Subset of channel binding fields needed by the bot.
#[derive(Debug, Deserialize)]
pub(crate) struct ChannelBindingResponse {
    pub session_key: String,
}

/// Response from `POST /sessions/{key}/send`.
#[derive(Debug, Deserialize)]
pub(crate) struct ChatMessageResponse {
    pub message: ChatMessageData,
}

/// Minimal representation of a chat message for bot consumption.
#[derive(Debug, Deserialize)]
pub(crate) struct ChatMessageData {
    #[allow(dead_code)]
    pub role:    String,
    pub content: serde_json::Value,
}

/// A single session in the list returned by `GET /api/v1/chat/sessions`.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(crate) struct SessionListItem {
    pub key:           String,
    pub title:         Option<String>,
    pub message_count: i64,
    pub updated_at:    String,
}

/// Session detail returned by `GET /api/v1/chat/sessions/{key}`.
#[derive(Debug, Deserialize)]
pub(crate) struct SessionDetailResponse {
    pub key:           String,
    pub title:         Option<String>,
    pub model:         Option<String>,
    pub message_count: i64,
    pub preview:       Option<String>,
    pub created_at:    String,
    pub updated_at:    String,
}

/// MCP server info returned by `GET /api/v1/mcp/servers`.
#[derive(Debug, Deserialize)]
pub(crate) struct McpServerInfo {
    pub name:   String,
    pub status: McpServerStatus,
}

/// MCP server connection status.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub(crate) enum McpServerStatus {
    Connected,
    Connecting,
    Disconnected,
    Error { message: String },
}

/// SSE event from the streaming chat endpoint.
///
/// Mirrors `rara_domain_chat::stream::ChatStreamEvent` -- duplicated here
/// so the bot crate doesn't depend on the domain crate.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ChatStreamEvent {
    TextDelta {
        text: String,
    },
    ReasoningDelta {
        #[allow(dead_code)]
        text: String,
    },
    Thinking,
    ThinkingDone,
    Iteration {
        #[allow(dead_code)]
        index: usize,
    },
    ToolCallStart {
        #[allow(dead_code)]
        id: String,
        #[allow(dead_code)]
        name: String,
    },
    ToolCallEnd {
        #[allow(dead_code)]
        id: String,
        #[allow(dead_code)]
        name: String,
        success: bool,
        #[allow(dead_code)]
        error: Option<String>,
    },
    Done {
        text: String,
    },
    Error {
        message: String,
    },
}

/// Response from dispatching a coding task.
#[derive(Debug, Deserialize)]
pub(crate) struct CodingTaskResponse {
    pub id:           String,
    pub branch:       String,
    pub tmux_session: String,
    pub status:       String,
}

/// Summary of a coding task from the list endpoint.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(crate) struct CodingTaskSummaryResponse {
    pub id:         String,
    pub status:     String,
    pub agent_type: String,
    pub branch:     String,
    pub prompt:     String,
    pub pr_url:     Option<String>,
}

impl ChatMessageData {
    /// Extract text content from the message, handling both plain string
    /// and multimodal content block formats.
    pub fn text_content(&self) -> String {
        match &self.content {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Array(blocks) => blocks
                .iter()
                .filter_map(|b| {
                    if b.get("type")?.as_str()? == "text" {
                        b.get("text")?.as_str().map(String::from)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        }
    }
}
