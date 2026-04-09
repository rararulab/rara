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

//! Schedule tools — LLM-callable tools for scheduling future agent turns.
//!
//! Five tools are provided:
//! - `schedule-once` — trigger a future LLM task once after N seconds
//! - `schedule-interval` — trigger a future LLM task every N seconds
//! - `schedule-cron` — trigger a future LLM task from a cron expression
//! - `schedule-remove` — remove a scheduled job by ID
//! - `schedule-list` — list all scheduled jobs across sessions

use async_trait::async_trait;
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::oneshot;
use tracing::debug;

use crate::{
    event::{KernelEventEnvelope, Syscall},
    schedule::{JobId, JobWheel, Trigger},
    tool::{EmptyParams, ToolContext, ToolExecute},
};

// -- shared helper ------------------------------------------------------------

async fn register_job(
    trigger: Trigger,
    message: String,
    tags: Vec<String>,
    context: &ToolContext,
) -> anyhow::Result<Value> {
    let next_at = trigger.next_at();

    let event_queue = &context.event_queue;
    let session_key = context.session_key;

    let (tx, rx) = oneshot::channel();
    let _ = event_queue.push(KernelEventEnvelope::session_command(
        session_key,
        Syscall::RegisterJob {
            trigger,
            message,
            tags,
            reply_tx: tx,
        },
    ));

    let job_id = rx
        .await
        .map_err(|_| anyhow::anyhow!("kernel dropped reply channel"))?
        .map_err(|e| anyhow::anyhow!("register job failed: {e}"))?;

    debug!(job_id = %job_id, "scheduled job registered via tool");

    Ok(serde_json::json!({
        "job_id": job_id.to_string(),
        "next_run": next_at.to_string(),
    }))
}

// ============================================================================
// ScheduleOnceTool
// ============================================================================

/// Tool for scheduling a one-shot future agent turn.
#[derive(ToolDef)]
#[tool(
    name = "schedule-once",
    description = "Schedule a one-shot future agent turn. `message` is the prompt that will be \
                   sent to the LLM when the job fires, so write it like a user instruction and \
                   name any required skills explicitly.",
    tier = "deferred"
)]
pub struct ScheduleOnceTool;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScheduleOnceParams {
    /// Fire once after this many seconds
    after_seconds: u64,
    /// Prompt for the future scheduled agent. This is sent to the LLM when the
    /// job fires, so write it like a user instruction; if a skill should be
    /// used, name it explicitly.
    message:       String,
    /// Routing tags for task report notification matching
    /// (e.g. `["pr_review", "repo:rararulab/rara"]`)
    #[serde(default)]
    tags:          Vec<String>,
}

#[async_trait]
impl ToolExecute for ScheduleOnceTool {
    type Output = Value;
    type Params = ScheduleOnceParams;

    async fn run(&self, p: ScheduleOnceParams, context: &ToolContext) -> anyhow::Result<Value> {
        if p.after_seconds == 0 {
            return Err(anyhow::anyhow!("after_seconds must be > 0"));
        }

        let now = jiff::Timestamp::now();
        let run_at = now
            .checked_add(jiff::SignedDuration::from_secs(p.after_seconds as i64))
            .map_err(|e| anyhow::anyhow!("timestamp overflow: {e}"))?;

        register_job(Trigger::Once { run_at }, p.message, p.tags, context).await
    }
}

// ============================================================================
// ScheduleIntervalTool
// ============================================================================

/// Tool for scheduling a repeating future agent turn.
#[derive(ToolDef)]
#[tool(
    name = "schedule-interval",
    description = "Schedule a repeating future agent turn. `message` is the prompt that will be \
                   sent to the LLM each time the job fires, so write it like a user instruction \
                   and name any required skills explicitly.",
    tier = "deferred"
)]
pub struct ScheduleIntervalTool;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScheduleIntervalParams {
    /// Fire every N seconds (repeating)
    interval_seconds: u64,
    /// Prompt for the future scheduled agent. This is sent to the LLM every
    /// time the job fires, so write it like a user instruction; if a skill
    /// should be used, name it explicitly.
    message:          String,
    /// Routing tags for task report notification matching
    /// (e.g. `["pr_review", "repo:rararulab/rara"]`)
    #[serde(default)]
    tags:             Vec<String>,
}

#[async_trait]
impl ToolExecute for ScheduleIntervalTool {
    type Output = Value;
    type Params = ScheduleIntervalParams;

    async fn run(&self, p: ScheduleIntervalParams, context: &ToolContext) -> anyhow::Result<Value> {
        if p.interval_seconds == 0 {
            return Err(anyhow::anyhow!("interval_seconds must be > 0"));
        }

        let now = jiff::Timestamp::now();
        let next_at = now
            .checked_add(jiff::SignedDuration::from_secs(p.interval_seconds as i64))
            .map_err(|e| anyhow::anyhow!("timestamp overflow: {e}"))?;

        register_job(
            Trigger::Interval {
                every_secs: p.interval_seconds,
                next_at,
            },
            p.message,
            p.tags,
            context,
        )
        .await
    }
}

// ============================================================================
// ScheduleCronTool
// ============================================================================

/// Tool for scheduling a future agent turn using a cron expression.
#[derive(ToolDef)]
#[tool(
    name = "schedule-cron",
    description = "Schedule a future agent turn using a 6-field cron expression: 'sec min hour \
                   day month weekday'. `message` is the prompt that will be sent to the LLM \
                   whenever the job fires, so write it like a user instruction and name any \
                   required skills explicitly.",
    tier = "deferred"
)]
pub struct ScheduleCronTool;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScheduleCronParams {
    /// 6-field cron expression: 'sec min hour day month weekday' (e.g. '0 0 9 *
    /// * *' for daily at 9am UTC, '0 */5 * * * *' for every 5 minutes)
    cron:    String,
    /// Prompt for the future scheduled agent. This is sent to the LLM whenever
    /// the cron job fires, so write it like a user instruction; if a skill
    /// should be used, name it explicitly.
    message: String,
    /// Routing tags for task report notification matching
    /// (e.g. `["pr_review", "repo:rararulab/rara"]`)
    #[serde(default)]
    tags:    Vec<String>,
}

#[async_trait]
impl ToolExecute for ScheduleCronTool {
    type Output = Value;
    type Params = ScheduleCronParams;

    async fn run(&self, p: ScheduleCronParams, context: &ToolContext) -> anyhow::Result<Value> {
        if p.cron.is_empty() {
            return Err(anyhow::anyhow!("cron expression must not be empty"));
        }

        // Validate the expression up front: parse error AND
        // "syntactically valid but yields no future fire time" both turn
        // into a single user-actionable error here, rather than the silent
        // delete that drain_expired used to perform.
        let now = jiff::Timestamp::now();
        let next_at = JobWheel::next_cron_time(&p.cron, now).ok_or_else(|| {
            anyhow::anyhow!(
                "cron expression '{}' is invalid or yields no future fire time. Check for \
                 impossible dates like 'Feb 31' or expressions whose last valid time has passed.",
                p.cron
            )
        })?;

        register_job(
            Trigger::Cron {
                expr: p.cron,
                next_at,
            },
            p.message,
            p.tags,
            context,
        )
        .await
    }
}

// ============================================================================
// ScheduleRemoveTool
// ============================================================================

/// Tool for removing a scheduled task by ID.
#[derive(ToolDef)]
#[tool(
    name = "schedule-remove",
    description = "Remove a previously scheduled task by its job ID.",
    tier = "deferred"
)]
pub struct ScheduleRemoveTool;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScheduleRemoveParams {
    /// The ID of the job to remove
    job_id: String,
}

#[async_trait]
impl ToolExecute for ScheduleRemoveTool {
    type Output = Value;
    type Params = ScheduleRemoveParams;

    async fn run(&self, p: ScheduleRemoveParams, context: &ToolContext) -> anyhow::Result<Value> {
        let event_queue = &context.event_queue;
        let session_key = context.session_key;

        let job_id = JobId::try_from_raw(&p.job_id)
            .map_err(|e| anyhow::anyhow!("invalid job_id '{}': {e}", p.job_id))?;

        let (tx, rx) = oneshot::channel();
        let _ = event_queue.push(KernelEventEnvelope::session_command(
            session_key,
            Syscall::RemoveJob {
                job_id,
                reply_tx: tx,
            },
        ));

        rx.await
            .map_err(|_| anyhow::anyhow!("kernel dropped reply channel"))?
            .map_err(|e| anyhow::anyhow!("remove job failed: {e}"))?;

        Ok(serde_json::json!({ "ok": true }))
    }
}

// ============================================================================
// ScheduleListTool
// ============================================================================

/// Tool for listing all scheduled tasks across sessions.
#[derive(ToolDef)]
#[tool(
    name = "schedule-list",
    description = "List all scheduled tasks across sessions.",
    tier = "deferred"
)]
pub struct ScheduleListTool;

#[async_trait]
impl ToolExecute for ScheduleListTool {
    type Output = Value;
    type Params = EmptyParams;

    async fn run(&self, _p: EmptyParams, context: &ToolContext) -> anyhow::Result<Value> {
        let event_queue = &context.event_queue;
        let session_key = context.session_key;

        let (tx, rx) = oneshot::channel();
        let _ = event_queue.push(KernelEventEnvelope::session_command(
            session_key,
            Syscall::ListJobs { reply_tx: tx },
        ));

        let jobs = rx
            .await
            .map_err(|_| anyhow::anyhow!("kernel dropped reply channel"))?
            .map_err(|e| anyhow::anyhow!("list jobs failed: {e}"))?;

        let list: Vec<serde_json::Value> = jobs
            .iter()
            .map(|j| {
                serde_json::json!({
                    "job_id": j.id.to_string(),
                    "trigger": j.trigger,
                    "message": j.message,
                    "created_at": j.created_at.to_string(),
                })
            })
            .collect();

        Ok(serde_json::json!({
            "jobs": list,
            "count": list.len(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{
        io::MessageId,
        queue::{ShardedEventQueue, ShardedEventQueueConfig, ShardedQueueRef},
        session::SessionKey,
    };

    fn build_queue() -> ShardedQueueRef {
        Arc::new(ShardedEventQueue::new(ShardedEventQueueConfig {
            num_shards:      0,
            shard_capacity:  1,
            global_capacity: 16,
        }))
    }

    fn build_context() -> ToolContext {
        ToolContext {
            user_id:               "test-user".into(),
            session_key:           SessionKey::new(),
            origin_endpoint:       None,
            event_queue:           build_queue(),
            rara_message_id:       MessageId::new(),
            context_window_tokens: 0,
            tool_registry:         None,
            stream_handle:         None,
            tool_call_id:          None,
        }
    }

    /// A cron expression that parses but yields no future fire time
    /// (e.g., Feb 31 — sec min hour day month weekday) must be rejected at
    /// registration, not silently swallowed by the wheel.
    #[tokio::test]
    async fn cron_with_no_future_time_rejected_at_registration() {
        let tool = ScheduleCronTool;
        let ctx = build_context();
        let params = ScheduleCronParams {
            cron:    "0 0 0 31 2 *".into(),
            message: "should never fire".into(),
            tags:    vec![],
        };

        let err = tool
            .run(params, &ctx)
            .await
            .expect_err("impossible cron must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("no future fire time") || msg.contains("invalid"),
            "error should explain why the cron was rejected, got: {msg}"
        );
    }

    /// A clearly valid 6-field cron expression must pass validation.
    ///
    /// We can't drive the success path all the way through `register_job`
    /// in a unit test (no kernel is draining the queue), so we exercise the
    /// validator directly via [`crate::schedule::JobWheel::next_cron_time`]
    /// — which is exactly what the tool calls before pushing the syscall.
    #[test]
    fn valid_cron_accepted() {
        let now = jiff::Timestamp::now();
        let next = JobWheel::next_cron_time("*/5 * * * * *", now);
        assert!(
            next.is_some(),
            "*/5 * * * * * must yield a future fire time"
        );
        let next = next.unwrap();
        assert!(next > now, "next fire time must be strictly after now");
    }
}
