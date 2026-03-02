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

//! Callback handlers for inline keyboard interactions.
//!
//! - [`SessionSwitchCallbackHandler`]: handles `switch:{session_key}`
//!   callbacks.
//! - [`SearchPaginationCallbackHandler`]: handles
//!   `search_more:{count}:{params}` callbacks.

use std::sync::Arc;

use async_trait::async_trait;
use rara_kernel::{
    channel::command::{CallbackContext, CallbackHandler, CallbackResult},
    error::KernelError,
};

use super::{
    client::BotServiceClient,
    job::{decode_search_params, encode_search_params, format_job_results},
};

// ---------------------------------------------------------------------------
// SessionSwitchCallbackHandler
// ---------------------------------------------------------------------------

/// Handles `switch:{session_key}` callback queries from the `/sessions`
/// inline keyboard.
pub struct SessionSwitchCallbackHandler {
    client: Arc<dyn BotServiceClient>,
}

impl SessionSwitchCallbackHandler {
    pub fn new(client: Arc<dyn BotServiceClient>) -> Self { Self { client } }
}

#[async_trait]
impl CallbackHandler for SessionSwitchCallbackHandler {
    fn prefix(&self) -> &str { "switch:" }

    async fn handle(&self, context: &CallbackContext) -> Result<CallbackResult, KernelError> {
        let session_key = &context.data["switch:".len()..];
        let chat_id = extract_chat_id(context);
        let account = extract_bot_username(context);

        match self
            .client
            .bind_channel("telegram", &account, &chat_id, session_key)
            .await
        {
            Ok(_) => Ok(CallbackResult::SendMessage {
                text: format!(
                    "Switched to session: <code>{}</code>",
                    html_escape(session_key)
                ),
            }),
            Err(e) => Ok(CallbackResult::SendMessage {
                text: format!("Failed to switch session: {e}"),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// SearchPaginationCallbackHandler
// ---------------------------------------------------------------------------

/// Handles `search_more:{count}:{params}` callback queries for job search
/// pagination ("Load More" button).
pub struct SearchPaginationCallbackHandler {
    client: Arc<dyn BotServiceClient>,
}

impl SearchPaginationCallbackHandler {
    pub fn new(client: Arc<dyn BotServiceClient>) -> Self { Self { client } }
}

#[async_trait]
impl CallbackHandler for SearchPaginationCallbackHandler {
    fn prefix(&self) -> &str { "search_more:" }

    async fn handle(&self, context: &CallbackContext) -> Result<CallbackResult, KernelError> {
        // Parse: "search_more:{count}:{encoded_params}"
        let payload = &context.data["search_more:".len()..];
        let parts: Vec<&str> = payload.splitn(2, ':').collect();
        if parts.len() != 2 {
            return Ok(CallbackResult::Ack);
        }

        let current_count: u32 = match parts[0].parse() {
            Ok(n) => n,
            Err(_) => return Ok(CallbackResult::Ack),
        };

        let (keywords, location) = decode_search_params(parts[1]);
        let new_max = current_count + 3;

        let jobs = match self
            .client
            .discover_jobs(keywords.clone(), location.clone(), new_max)
            .await
        {
            Ok(jobs) => jobs,
            Err(e) => {
                return Ok(CallbackResult::SendMessage {
                    text: format!("Load more failed: {e}"),
                });
            }
        };

        let mut text = format_job_results(&jobs, &keywords, location.as_deref());

        // If we got fewer results than requested, there are no more results.
        // Otherwise, append a hint about the Load More button.
        #[allow(clippy::cast_possible_truncation)]
        if (jobs.len() as u32) >= new_max {
            let encoded = encode_search_params(&keywords, location.as_deref());
            // The adapter cannot render a new keyboard from EditMessage alone.
            // We include the "Load More" hint in the text instead.
            text.push_str(&format!(
                "\n<i>Use the button below to load more results.</i>\n<!-- \
                 search_more:{}:{encoded} -->",
                jobs.len(),
            ));
        }

        Ok(CallbackResult::EditMessage { text })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn extract_chat_id(context: &CallbackContext) -> String {
    context
        .metadata
        .get("telegram_chat_id")
        .and_then(|v| {
            v.as_i64()
                .map(|n| n.to_string())
                .or_else(|| v.as_str().map(String::from))
        })
        .unwrap_or_else(|| "0".to_owned())
}

fn extract_bot_username(context: &CallbackContext) -> String {
    context
        .metadata
        .get("telegram_bot_username")
        .and_then(|v| v.as_str())
        .unwrap_or("default")
        .to_owned()
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use rara_kernel::channel::types::{ChannelType, ChannelUser};

    use super::*;
    use crate::telegram::commands::client::{
        BotServiceError, ChannelBinding, CodingTask, CodingTaskSummary, DiscoveryJob,
        McpServerInfo, SessionDetail, SessionListItem,
    };

    struct MockCallbackClient;

    #[async_trait]
    impl BotServiceClient for MockCallbackClient {
        async fn get_channel_session(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Option<ChannelBinding>, BotServiceError> {
            Ok(None)
        }

        async fn bind_channel(
            &self,
            _: &str,
            _: &str,
            _: &str,
            k: &str,
        ) -> Result<ChannelBinding, BotServiceError> {
            Ok(ChannelBinding {
                session_key: k.to_owned(),
            })
        }

        async fn create_session(&self, _: &str, _: Option<&str>) -> Result<(), BotServiceError> {
            Ok(())
        }

        async fn clear_session_messages(&self, _: &str) -> Result<(), BotServiceError> { Ok(()) }

        async fn list_sessions(&self, _: u32) -> Result<Vec<SessionListItem>, BotServiceError> {
            Ok(vec![])
        }

        async fn get_session(&self, _: &str) -> Result<SessionDetail, BotServiceError> {
            Err(BotServiceError::Service {
                message: "n/a".into(),
            })
        }

        async fn update_session(
            &self,
            _: &str,
            _: Option<&str>,
        ) -> Result<SessionDetail, BotServiceError> {
            Err(BotServiceError::Service {
                message: "n/a".into(),
            })
        }

        async fn discover_jobs(
            &self,
            _: Vec<String>,
            _: Option<String>,
            max: u32,
        ) -> Result<Vec<DiscoveryJob>, BotServiceError> {
            // Return exactly `max` jobs to simulate "more available".
            let mut jobs = Vec::new();
            for i in 0..max {
                jobs.push(DiscoveryJob {
                    title:           format!("Job {i}"),
                    company:         format!("Company {i}"),
                    location:        Some("Remote".to_owned()),
                    url:             None,
                    salary_min:      None,
                    salary_max:      None,
                    salary_currency: None,
                });
            }
            Ok(jobs)
        }

        async fn submit_jd_parse(&self, _: &str) -> Result<(), BotServiceError> { Ok(()) }

        async fn list_mcp_servers(&self) -> Result<Vec<McpServerInfo>, BotServiceError> {
            Ok(vec![])
        }

        async fn get_mcp_server(&self, _: &str) -> Result<McpServerInfo, BotServiceError> {
            Err(BotServiceError::Service {
                message: "n/a".into(),
            })
        }

        async fn add_mcp_server(
            &self,
            _: &str,
            _: &str,
            _: &[String],
        ) -> Result<McpServerInfo, BotServiceError> {
            Err(BotServiceError::Service {
                message: "n/a".into(),
            })
        }

        async fn start_mcp_server(&self, _: &str) -> Result<(), BotServiceError> { Ok(()) }

        async fn remove_mcp_server(&self, _: &str) -> Result<(), BotServiceError> { Ok(()) }

        async fn dispatch_coding_task(
            &self,
            _: &str,
            _: &str,
        ) -> Result<CodingTask, BotServiceError> {
            Err(BotServiceError::Service {
                message: "n/a".into(),
            })
        }

        async fn list_coding_tasks(&self) -> Result<Vec<CodingTaskSummary>, BotServiceError> {
            Ok(vec![])
        }
    }

    fn make_callback_context(data: &str) -> CallbackContext {
        let mut metadata = HashMap::new();
        metadata.insert("telegram_chat_id".to_owned(), serde_json::json!(123));
        metadata.insert(
            "telegram_bot_username".to_owned(),
            serde_json::json!("test_bot"),
        );
        CallbackContext {
            channel_type: ChannelType::Telegram,
            session_key: "tg:123".to_owned(),
            user: ChannelUser {
                platform_id:  "123".to_owned(),
                display_name: Some("Test".to_owned()),
            },
            data: data.to_owned(),
            message_id: Some("100".to_owned()),
            metadata,
        }
    }

    #[tokio::test]
    async fn session_switch_binds_channel() {
        let handler = SessionSwitchCallbackHandler::new(Arc::new(MockCallbackClient));
        assert_eq!(handler.prefix(), "switch:");

        let ctx = make_callback_context("switch:tg-123-999");
        let result = handler.handle(&ctx).await;
        match result {
            Ok(CallbackResult::SendMessage { text }) => {
                assert!(text.contains("tg-123-999"));
            }
            other => panic!("expected SendMessage, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn search_pagination_loads_more() {
        let handler = SearchPaginationCallbackHandler::new(Arc::new(MockCallbackClient));
        assert_eq!(handler.prefix(), "search_more:");

        let ctx = make_callback_context("search_more:3:rust+engineer@remote");
        let result = handler.handle(&ctx).await;
        match result {
            Ok(CallbackResult::EditMessage { text }) => {
                // Should have 6 jobs (3 existing + 3 new).
                assert!(text.contains("Job 0"));
                assert!(text.contains("Job 5"));
            }
            other => panic!("expected EditMessage, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn search_pagination_invalid_data() {
        let handler = SearchPaginationCallbackHandler::new(Arc::new(MockCallbackClient));
        let ctx = make_callback_context("search_more:invalid");
        let result = handler.handle(&ctx).await;
        match result {
            Ok(CallbackResult::Ack) => {}
            other => panic!("expected Ack, got {other:?}"),
        }
    }
}
