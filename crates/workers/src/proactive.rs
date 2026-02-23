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
use rara_agents::builtin::proactive::ProactiveAgent;
use rara_sessions::types::SessionKey;
use tracing::{info, warn};

use crate::worker_state::AppState;

/// Fixed session key for all proactive agent interactions.
const PROACTIVE_SESSION_KEY: &str = "agent:proactive";

/// Maximum number of historical messages loaded for context.
const PROACTIVE_HISTORY_LIMIT: i64 = 50;

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

        // 2. Ensure proactive session exists and load history
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

        // 3. Delegate to ProactiveAgent
        let agent = ProactiveAgent::new(state.orchestrator.clone());
        let output = agent
            .run(&activity_summary, &history)
            .await
            .map_err(|e| WorkError::transient(format!("agent run failed: {e}")))?;

        let response_text = output.response_text;

        // 4. Persist conversation turns to the proactive session
        //    Reconstruct the user prompt for persistence (mirrors ProactiveAgent).
        let user_prompt = format!(
            "以下是最近24小时的用户活动摘要：\n\n{}\n\n根据你的行为策略，\
             决定是否需要主动联系用户。\n你可以使用工具查询更多信息、发送通知、或安排后续任务。\
             \n如果没有值得做的事情，直接回复 DONE。",
            activity_summary
        );
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

        // 5. Log outcome
        if response_text == "DONE" || response_text == "SKIP" || response_text.is_empty() {
            info!("proactive agent: nothing to report");
        } else {
            info!(
                iterations = output.iterations,
                tool_calls = output.tool_calls_made,
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
