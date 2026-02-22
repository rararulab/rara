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
//! and takes autonomous actions via a multi-turn agent loop with full tools.

use async_trait::async_trait;
use chrono::Utc;
use common_worker::{FallibleWorker, WorkError, WorkResult, WorkerContext};
use rara_agents::runner::{AgentRunner, UserContent};
use rara_domain_chat::service::to_chat_message;
use rara_domain_shared::settings::model::Settings;
use rara_sessions::types::SessionKey;
use tracing::{info, warn};

use crate::worker_state::AppState;

/// Fixed session key for all proactive agent interactions.
const PROACTIVE_SESSION_KEY: &str = "agent:proactive";

/// Maximum number of agent loop iterations per proactive run.
const PROACTIVE_MAX_ITERATIONS: usize = 15;

/// Maximum number of historical messages loaded for context.
const PROACTIVE_HISTORY_LIMIT: i64 = 50;

/// Default agent behavior policy embedded into the binary.
const DEFAULT_AGENT_POLICY: &str = include_str!("../../../prompts/workers/agent_policy.md");

fn compose_policy(base_policy: &str, soul_prompt: Option<&str>) -> String {
    if let Some(soul) = soul_prompt.filter(|s| !s.trim().is_empty()) {
        return format!("{soul}\n\n# Operational Policy\n{base_policy}");
    }
    base_policy.to_owned()
}

fn resolve_soul_prompt(settings: &Settings) -> Option<String> {
    if settings
        .agent
        .soul
        .as_deref()
        .is_some_and(|s| !s.trim().is_empty())
    {
        return settings.agent.soul.clone();
    }
    let markdown_soul = rara_paths::load_agent_soul_prompt();
    if markdown_soul.trim().is_empty() {
        return None;
    }
    Some(markdown_soul)
}

/// Load the agent behavior policy from settings, file, or built-in default.
pub async fn load_agent_policy(settings: &Settings) -> String {
    let soul_prompt = resolve_soul_prompt(settings);
    // 1. On-disk policy file
    let policy_path = rara_paths::agent_policy_file();
    if let Ok(content) = tokio::fs::read_to_string(policy_path).await
        && !content.trim().is_empty()
    {
        return compose_policy(&content, soul_prompt.as_deref());
    }

    let prompt_content =
        rara_paths::load_prompt_markdown("workers/agent_policy.md", DEFAULT_AGENT_POLICY);
    if !prompt_content.trim().is_empty() {
        return compose_policy(&prompt_content, soul_prompt.as_deref());
    }
    // 2. Built-in default
    compose_policy(DEFAULT_AGENT_POLICY, soul_prompt.as_deref())
}

/// Background worker that reviews recent chat sessions and proactively
/// takes action via a multi-turn agent loop with full tool access.
pub struct ProactiveAgentWorker;

#[async_trait]
impl FallibleWorker<AppState> for ProactiveAgentWorker {
    async fn work(&mut self, ctx: WorkerContext<AppState>) -> WorkResult {
        let state = ctx.state();
        let settings = state.settings_svc.current();

        // Guard: AI not configured
        if !settings.ai.is_configured() {
            warn!("proactive agent skipped: AI not configured");
            return Ok(());
        }

        // Guard: Telegram not configured
        if settings.telegram.chat_id.is_none() {
            warn!("proactive agent skipped: Telegram chat_id not configured");
            return Ok(());
        }

        // 1. Collect activity summary from recent sessions
        let activity_summary = collect_activity_summary(state).await?;
        if activity_summary.is_empty() {
            info!("proactive agent: no recent activity");
            return Ok(());
        }

        // 2. Load agent behavior policy
        let policy = load_agent_policy(&settings).await;

        // 3. Ensure proactive session exists and load history
        let session_key = SessionKey::from_raw(PROACTIVE_SESSION_KEY);
        let _ = state
            .chat_service
            .ensure_session(&session_key, Some("Proactive Agent"), None, None)
            .await;
        let history = state
            .chat_service
            .get_messages(&session_key, None, Some(PROACTIVE_HISTORY_LIMIT))
            .await
            .unwrap_or_default();

        // 4. Build user prompt with activity summary
        let user_prompt = format!(
            "以下是最近24小时的用户活动摘要：\n\n{}\n\n根据你的行为策略，\
             决定是否需要主动联系用户。\n你可以使用工具查询更多信息、发送通知、或安排后续任务。\\
             n如果没有值得做的事情，直接回复 DONE。",
            activity_summary
        );

        // 5. Resolve model
        let model = settings
            .ai
            .model_for(rara_domain_shared::settings::model::ModelScenario::Chat)
            .to_owned();

        // 6. Convert history to async-openai message format
        let chat_history: Vec<_> = history.iter().map(to_chat_message).collect();

        // 7. Build and run multi-turn AgentRunner with full tools
        let tools = state.chat_service.tools().clone();

        let runner = AgentRunner::builder()
            .llm_provider(state.llm_provider.clone())
            .model_name(model)
            .system_prompt(policy)
            .user_content(UserContent::Text(user_prompt.clone()))
            .history(chat_history)
            .max_iterations(PROACTIVE_MAX_ITERATIONS)
            .build();

        let result = runner
            .run(&tools, None)
            .await
            .map_err(|e| WorkError::transient(format!("agent run failed: {e}")))?;

        // 8. Extract assistant response
        let response_text = result
            .provider_response
            .choices
            .first()
            .and_then(|c| c.message.content.as_deref())
            .unwrap_or("")
            .trim()
            .to_owned();

        // 9. Persist conversation turns to the proactive session
        state
            .chat_service
            .append_message_raw(
                &session_key,
                &rara_sessions::types::ChatMessage::user(&user_prompt),
            )
            .await
            .ok();
        state
            .chat_service
            .append_message_raw(
                &session_key,
                &rara_sessions::types::ChatMessage::assistant(&response_text),
            )
            .await
            .ok();

        // 10. Log outcome
        if response_text == "DONE" || response_text == "SKIP" || response_text.is_empty() {
            info!("proactive agent: nothing to report");
        } else {
            info!(
                iterations = result.iterations,
                tool_calls = result.tool_calls_made,
                response_len = response_text.len(),
                "proactive agent completed: {}",
                &response_text[..response_text.len().min(200)]
            );
        }

        Ok(())
    }
}

/// Collect a summary of recent chat activity (last 24 hours).
async fn collect_activity_summary(state: &AppState) -> Result<String, WorkError> {
    let sessions = state
        .chat_service
        .list_sessions(Some(20), None)
        .await
        .map_err(|e| WorkError::transient(format!("list sessions: {e}")))?;

    let cutoff = Utc::now() - chrono::Duration::hours(24);
    let recent_sessions: Vec<_> = sessions
        .into_iter()
        .filter(|s| s.updated_at > cutoff)
        // Exclude the proactive agent's own session from activity summary
        .filter(|s| s.key.as_str() != PROACTIVE_SESSION_KEY)
        .collect();

    if recent_sessions.is_empty() {
        return Ok(String::new());
    }

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

        activity_summary.push_str(&format!("\nSession \"{}\" ({} messages):\n", title, count));
        for msg in &messages {
            let role = msg.role.to_string();
            let text = msg.content.as_text();
            // Truncate long messages (char-boundary safe)
            let truncated: String = if text.chars().count() > 200 {
                text.chars().take(200).collect()
            } else {
                text
            };
            activity_summary.push_str(&format!("  {role}: {truncated}\n"));
        }
    }

    Ok(activity_summary)
}
