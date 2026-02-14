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

//! Proactive agent worker that periodically reviews recent chat activity
//! and sends encouraging Telegram messages when warranted.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use common_worker::{FallibleWorker, WorkError, WorkResult, WorkerContext};
use openrouter_rs::api::chat::Content;
use rara_agents::runner::AgentRunner;
use rara_agents::tool_registry::ToolRegistry;
use rara_domain_shared::notify::types::{NotificationPriority, SendTelegramNotificationRequest};
use tracing::{info, warn};

use crate::worker_state::AppState;

const DEFAULT_SOUL_PROMPT: &str = "\
You are a proactive job search companion. You're encouraging, data-driven, and concise.
When reviewing recent user activity, consider:
- Did they share JDs but not follow up?
- Are they stuck on applications without progress?
- Did they ask questions that suggest uncertainty?
If you spot something worth mentioning, craft a brief, warm Telegram message (max 300 chars).
If nothing stands out, respond with exactly \"SKIP\".";

/// Background worker that reviews recent chat sessions and proactively
/// sends motivational or actionable Telegram messages.
pub struct ProactiveAgentWorker;

#[async_trait]
impl FallibleWorker<AppState> for ProactiveAgentWorker {
    async fn work(&mut self, ctx: WorkerContext<AppState>) -> WorkResult {
        let state = ctx.state();
        let settings = state.settings_svc.current();

        // Guard: proactive disabled
        if !settings.agent.proactive_enabled {
            return Ok(());
        }

        // Guard: AI not configured
        if settings.ai.openrouter_api_key.is_none() {
            warn!("proactive agent skipped: AI not configured");
            return Ok(());
        }

        // Guard: Telegram not configured
        let chat_id = match settings.telegram.chat_id {
            Some(id) => id,
            None => {
                warn!("proactive agent skipped: Telegram chat_id not configured");
                return Ok(());
            }
        };

        // List recent sessions (updated in last 24h)
        let sessions = state
            .chat_service
            .list_sessions(Some(20), None)
            .await
            .map_err(|e| WorkError::transient(format!("list sessions: {e}")))?;

        let cutoff = Utc::now() - chrono::Duration::hours(24);
        let recent_sessions: Vec<_> =
            sessions.into_iter().filter(|s| s.updated_at > cutoff).collect();

        if recent_sessions.is_empty() {
            info!("proactive agent: no recent activity");
            return Ok(());
        }

        // Build activity summary
        let mut activity_summary = String::new();
        for session in &recent_sessions {
            let key = &session.key;
            let title = session.title.as_deref().unwrap_or("Untitled");
            let count = session.message_count;
            let after_seq = (count - 20).max(0);

            let messages = match state
                .chat_service
                .get_messages(key, Some(after_seq), None)
                .await
            {
                Ok(msgs) => msgs,
                Err(e) => {
                    warn!(session = %key, error = %e, "failed to read messages, skipping session");
                    continue;
                }
            };

            activity_summary.push_str(&format!(
                "\nSession \"{}\" ({} messages):\n",
                title, count
            ));
            for msg in &messages {
                let role = format!("{:?}", msg.role);
                let text = msg.content.as_text();
                // Truncate long messages (char-boundary safe)
                let truncated: String = if text.chars().count() > 200 {
                    text.chars().take(200).collect()
                } else {
                    text.to_owned()
                };
                activity_summary
                    .push_str(&format!("  {}: {}\n", role.to_lowercase(), &truncated));
            }
        }

        // Build reflection prompt
        let soul = settings
            .agent
            .soul
            .clone()
            .unwrap_or_else(|| DEFAULT_SOUL_PROMPT.to_owned());
        let model = settings
            .ai
            .model_for(rara_domain_shared::settings::model::ModelScenario::Chat)
            .to_owned();

        let user_prompt = format!(
            "--- Recent Activity (last 24h) ---\n{}\n--- Instructions ---\n\
             Based on the above activity, should you proactively reach out to the user?\n\
             If yes, write a brief Telegram message (max 300 chars, warm and helpful).\n\
             If nothing warrants a message, respond with exactly \"SKIP\".",
            activity_summary
        );

        let tools = Arc::new(ToolRegistry::default());

        let runner = AgentRunner::builder()
            .llm_provider(state.llm_provider.clone())
            .model_name(model)
            .system_prompt(soul)
            .user_content(Content::Text(user_prompt))
            .max_iterations(1_usize)
            .build();

        let result = runner.run(&tools, None).await.map_err(|e| {
            WorkError::transient(format!("agent run failed: {e}"))
        })?;

        let response_text = result
            .provider_response
            .choices
            .first()
            .and_then(|c| c.content())
            .unwrap_or("SKIP")
            .trim()
            .to_owned();

        if response_text == "SKIP" || response_text.is_empty() {
            info!("proactive agent: nothing to report");
            return Ok(());
        }

        // Send via Telegram
        info!(
            message_len = response_text.len(),
            "proactive agent: sending message"
        );
        state
            .notify_client
            .send_telegram(SendTelegramNotificationRequest {
                chat_id:        Some(chat_id),
                subject:        Some("Proactive Agent".to_owned()),
                body:           response_text,
                priority:       NotificationPriority::Normal,
                max_retries:    3,
                reference_type: Some("proactive_agent".to_owned()),
                reference_id:   None,
                metadata:       None,
            })
            .await
            .map_err(|e| WorkError::transient(format!("send telegram: {e}")))?;

        Ok(())
    }
}
