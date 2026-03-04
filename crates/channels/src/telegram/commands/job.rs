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

//! Job discovery commands: `/search` and `/jd`.

use std::{fmt::Write, sync::Arc};

use async_trait::async_trait;
use rara_kernel::{
    channel::{
        command::{CommandContext, CommandDefinition, CommandHandler, CommandInfo, CommandResult},
        types::InlineButton,
    },
    error::KernelError,
};

use super::client::{BotServiceClient, DiscoveryJob};

/// Handles `/search` and `/jd` commands.
pub struct JobCommandHandler {
    client: Arc<dyn BotServiceClient>,
}

impl JobCommandHandler {
    pub fn new(client: Arc<dyn BotServiceClient>) -> Self { Self { client } }
}

#[async_trait]
impl CommandHandler for JobCommandHandler {
    fn commands(&self) -> Vec<CommandDefinition> {
        vec![
            CommandDefinition {
                name:        "search".to_owned(),
                description: "Search for jobs".to_owned(),
                usage:       Some("/search <keywords> [@ location]".to_owned()),
            },
            CommandDefinition {
                name:        "jd".to_owned(),
                description: "Parse a Job Description".to_owned(),
                usage:       Some("/jd <text>".to_owned()),
            },
        ]
    }

    async fn handle(
        &self,
        command: &CommandInfo,
        context: &CommandContext,
    ) -> Result<CommandResult, KernelError> {
        match command.name.as_str() {
            "search" => self.handle_search(&command.args, context).await,
            "jd" => self.handle_jd(&command.args, context).await,
            _ => Ok(CommandResult::None),
        }
    }
}

impl JobCommandHandler {
    /// `/search <keywords> [@ location]` — search for jobs.
    async fn handle_search(
        &self,
        args: &str,
        context: &CommandContext,
    ) -> Result<CommandResult, KernelError> {
        // Primary chat check.
        if !is_primary_chat(context) {
            return Ok(CommandResult::Text(
                "This command is only available in the primary chat.".to_owned(),
            ));
        }

        let args = args.trim();
        if args.is_empty() {
            return Ok(CommandResult::Text(
                "Usage: /search <keywords> [@ location]\nExample: /search rust engineer @ beijing"
                    .to_owned(),
            ));
        }

        let (keywords_str, location) = if let Some(idx) = args.find(" @ ") {
            (&args[..idx], Some(args[idx + 3..].trim().to_owned()))
        } else {
            (args, None)
        };

        let keywords: Vec<String> = keywords_str.split_whitespace().map(String::from).collect();
        if keywords.is_empty() {
            return Ok(CommandResult::Text(
                "Please provide at least one keyword.".to_owned(),
            ));
        }

        let jobs = match self
            .client
            .discover_jobs(keywords.clone(), location.clone(), 3)
            .await
        {
            Ok(jobs) => jobs,
            Err(e) => {
                return Ok(CommandResult::Text(format!("Search failed: {e}")));
            }
        };

        if jobs.is_empty() {
            return Ok(CommandResult::Text(
                "No jobs found matching your criteria.".to_owned(),
            ));
        }

        let text = format_job_results(&jobs, &keywords, location.as_deref());
        let encoded = encode_search_params(&keywords, location.as_deref());
        let keyboard = vec![vec![InlineButton {
            text:          "Load More".to_owned(),
            callback_data: Some(format!("search_more:{}:{encoded}", jobs.len())),
            url:           None,
        }]];

        Ok(CommandResult::HtmlWithKeyboard {
            html: text,
            keyboard,
        })
    }

    /// `/jd <text>` — submit JD text for parsing.
    async fn handle_jd(
        &self,
        args: &str,
        context: &CommandContext,
    ) -> Result<CommandResult, KernelError> {
        if !is_primary_chat(context) {
            return Ok(CommandResult::Text(
                "This command is only available in the primary chat.".to_owned(),
            ));
        }

        let jd_text = args.trim();
        if jd_text.is_empty() {
            return Ok(CommandResult::Text(
                "Usage: /jd <paste job description text>".to_owned(),
            ));
        }

        if let Err(e) = self.client.submit_jd_parse(jd_text).await {
            return Ok(CommandResult::Text(format!("JD parse failed: {e}")));
        }

        Ok(CommandResult::Text(
            "Received your JD, processing...".to_owned(),
        ))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Check whether the command context indicates a primary chat.
///
/// Looks for `is_primary_chat: true` in metadata. If absent, defaults to true
/// for private chats (non-group).
fn is_primary_chat(context: &CommandContext) -> bool {
    if let Some(val) = context.metadata.get("is_primary_chat") {
        return val.as_bool().unwrap_or(true);
    }
    // Default: treat as primary if no metadata present.
    true
}

/// Format job results as HTML.
pub(crate) fn format_job_results(
    jobs: &[DiscoveryJob],
    keywords: &[String],
    location: Option<&str>,
) -> String {
    let location_display = location.unwrap_or("any");
    let kw_escaped = html_escape(&keywords.join(" "));
    let loc_escaped = html_escape(location_display);
    let mut text = format!(
        "Found <b>{}</b> jobs for <i>{kw_escaped}</i> @ <i>{loc_escaped}</i>:\n\n",
        jobs.len(),
    );

    for (i, job) in jobs.iter().enumerate() {
        let title = html_escape(&job.title);
        let company = html_escape(&job.company);
        let _ = write!(text, "<b>{}.</b> {title} - {company}\n", i + 1);
        if let Some(ref loc) = job.location {
            let _ = write!(text, "   {}", html_escape(loc));
        }
        if let (Some(min), Some(max)) = (job.salary_min, job.salary_max) {
            let currency = job.salary_currency.as_deref().unwrap_or("");
            let _ = write!(text, " | {min}-{max} {currency}");
        }
        text.push('\n');
        if let Some(ref url) = job.url {
            let _ = write!(text, "   {url}\n");
        }
        text.push('\n');
    }

    text
}

/// Encode search params for callback data: `keyword1+keyword2@location`.
pub(crate) fn encode_search_params(keywords: &[String], location: Option<&str>) -> String {
    let kw = keywords.join("+");
    match location {
        Some(loc) => format!("{kw}@{loc}"),
        None => kw,
    }
}

/// Decode search params from callback data.
pub(crate) fn decode_search_params(encoded: &str) -> (Vec<String>, Option<String>) {
    if let Some(idx) = encoded.find('@') {
        let kw = encoded[..idx].split('+').map(String::from).collect();
        let loc = encoded[idx + 1..].to_owned();
        (kw, Some(loc))
    } else {
        let kw = encoded.split('+').map(String::from).collect();
        (kw, None)
    }
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
        BotServiceError, ChannelBinding, McpServerInfo, SessionDetail, SessionListItem,
    };

    struct MockJobClient;

    #[async_trait]
    impl BotServiceClient for MockJobClient {
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

        async fn create_session(&self, _: Option<&str>) -> Result<String, BotServiceError> {
            Ok("mock-session-key".to_owned())
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
            _keywords: Vec<String>,
            _location: Option<String>,
            _max: u32,
        ) -> Result<Vec<DiscoveryJob>, BotServiceError> {
            Ok(vec![
                DiscoveryJob {
                    title:           "Rust Engineer".to_owned(),
                    company:         "Acme Corp".to_owned(),
                    location:        Some("Remote".to_owned()),
                    url:             Some("https://example.com/job1".to_owned()),
                    salary_min:      Some(100_000),
                    salary_max:      Some(150_000),
                    salary_currency: Some("USD".to_owned()),
                },
                DiscoveryJob {
                    title:           "Go Developer".to_owned(),
                    company:         "Beta Inc".to_owned(),
                    location:        None,
                    url:             None,
                    salary_min:      None,
                    salary_max:      None,
                    salary_currency: None,
                },
            ])
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
    }

    fn make_context() -> CommandContext {
        let mut metadata = HashMap::new();
        metadata.insert("telegram_chat_id".to_owned(), serde_json::json!(123));
        metadata.insert("is_primary_chat".to_owned(), serde_json::json!(true));
        CommandContext {
            channel_type: ChannelType::Telegram,
            session_key: "tg:123".to_owned(),
            user: ChannelUser {
                platform_id:  "123".to_owned(),
                display_name: Some("Test".to_owned()),
            },
            metadata,
        }
    }

    #[tokio::test]
    async fn search_returns_results_with_keyboard() {
        let handler = JobCommandHandler::new(Arc::new(MockJobClient));
        let cmd = CommandInfo {
            name: "search".to_owned(),
            args: "rust engineer @ remote".to_owned(),
            raw:  "/search rust engineer @ remote".to_owned(),
        };
        let result = handler.handle(&cmd, &make_context()).await;
        match result {
            Ok(CommandResult::HtmlWithKeyboard { html, keyboard }) => {
                assert!(html.contains("Rust Engineer"));
                assert!(html.contains("Acme Corp"));
                assert!(html.contains("Go Developer"));
                // Load More button.
                assert_eq!(keyboard.len(), 1);
                assert!(
                    keyboard[0][0]
                        .callback_data
                        .as_ref()
                        .unwrap()
                        .starts_with("search_more:")
                );
            }
            other => panic!("expected HtmlWithKeyboard, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn search_empty_args_shows_usage() {
        let handler = JobCommandHandler::new(Arc::new(MockJobClient));
        let cmd = CommandInfo {
            name: "search".to_owned(),
            args: String::new(),
            raw:  "/search".to_owned(),
        };
        let result = handler.handle(&cmd, &make_context()).await;
        match result {
            Ok(CommandResult::Text(text)) => {
                assert!(text.contains("Usage"));
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn search_blocked_in_non_primary_chat() {
        let handler = JobCommandHandler::new(Arc::new(MockJobClient));
        let cmd = CommandInfo {
            name: "search".to_owned(),
            args: "rust".to_owned(),
            raw:  "/search rust".to_owned(),
        };
        let mut ctx = make_context();
        ctx.metadata
            .insert("is_primary_chat".to_owned(), serde_json::json!(false));
        let result = handler.handle(&cmd, &ctx).await;
        match result {
            Ok(CommandResult::Text(text)) => {
                assert!(text.contains("only available in the primary chat"));
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn jd_submits_text() {
        let handler = JobCommandHandler::new(Arc::new(MockJobClient));
        let cmd = CommandInfo {
            name: "jd".to_owned(),
            args: "We are looking for a Rust engineer...".to_owned(),
            raw:  "/jd We are looking for a Rust engineer...".to_owned(),
        };
        let result = handler.handle(&cmd, &make_context()).await;
        match result {
            Ok(CommandResult::Text(text)) => {
                assert!(text.contains("processing"));
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn encode_decode_search_params_roundtrip() {
        let kw = vec!["rust".to_owned(), "engineer".to_owned()];
        let encoded = encode_search_params(&kw, Some("beijing"));
        assert_eq!(encoded, "rust+engineer@beijing");
        let (decoded_kw, decoded_loc) = decode_search_params(&encoded);
        assert_eq!(decoded_kw, kw);
        assert_eq!(decoded_loc.as_deref(), Some("beijing"));
    }

    #[test]
    fn encode_decode_without_location() {
        let kw = vec!["rust".to_owned()];
        let encoded = encode_search_params(&kw, None);
        assert_eq!(encoded, "rust");
        let (decoded_kw, decoded_loc) = decode_search_params(&encoded);
        assert_eq!(decoded_kw, kw);
        assert!(decoded_loc.is_none());
    }
}
