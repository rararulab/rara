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
    schedule::{JobId, Trigger},
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
                   name any required skills explicitly."
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
                   and name any required skills explicitly."
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
                   required skills explicitly."
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

        use std::str::FromStr;
        let schedule = cron::Schedule::from_str(&p.cron)
            .map_err(|e| anyhow::anyhow!("invalid cron expression '{}': {e}", p.cron))?;

        let now = jiff::Timestamp::now();
        let now_chrono = chrono::DateTime::<chrono::Utc>::from_timestamp(
            now.as_second(),
            now.subsec_nanosecond() as u32,
        )
        .ok_or_else(|| anyhow::anyhow!("failed to convert timestamp"))?;

        let next_chrono = schedule
            .upcoming(chrono::Utc)
            .find(|t| *t > now_chrono)
            .ok_or_else(|| anyhow::anyhow!("cron expression '{}' yields no future time", p.cron))?;

        let next_at = jiff::Timestamp::from_second(next_chrono.timestamp())
            .map_err(|e| anyhow::anyhow!("timestamp conversion error: {e}"))?;

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
    description = "Remove a previously scheduled task by its job ID."
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
    description = "List all scheduled tasks across sessions."
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
