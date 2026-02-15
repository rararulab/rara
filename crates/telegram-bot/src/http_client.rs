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
//! This client reuses domain request/response models
//! ([`DiscoveryCriteria`], [`DiscoveryJobResponse`]) to avoid payload drift
//! between the bot and the main service.
//!
//! # Endpoints
//!
//! | Method | Path                             | Purpose                          |
//! |--------|----------------------------------|----------------------------------|
//! | POST   | `/api/v1/jobs/discover`          | Search jobs with keyword filters |
//! | POST   | `/api/v1/internal/bot/jd-parse`  | Submit raw JD text for parsing   |

use rara_domain_job::types::DiscoveryCriteria;
pub use rara_domain_job::types::DiscoveryJobResponse;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use snafu::{ResultExt, Snafu};

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
