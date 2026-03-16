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

//! Schedule tools — LLM-callable tools for managing scheduled tasks.
//!
//! Five tools are provided:
//! - `schedule-once` — fire once after N seconds
//! - `schedule-interval` — fire every N seconds (repeating)
//! - `schedule-cron` — fire according to a cron expression
//! - `schedule-remove` — remove a scheduled job by ID
//! - `schedule-list` — list all scheduled jobs for the current session

use async_trait::async_trait;
use serde::Deserialize;
use tokio::sync::oneshot;
use tracing::debug;

use crate::{
    event::{KernelEventEnvelope, Syscall},
    schedule::{JobId, Trigger},
    tool::{AgentTool, ToolContext, ToolOutput},
};

// -- shared helper ------------------------------------------------------------

async fn register_job(
    trigger: Trigger,
    message: String,
    context: &ToolContext,
) -> anyhow::Result<ToolOutput> {
    let next_at = trigger.next_at();

    let event_queue = context
        .event_queue
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("no event queue in tool context"))?;
    let session_key = context
        .session_key
        .ok_or_else(|| anyhow::anyhow!("no session key in tool context"))?;

    let (tx, rx) = oneshot::channel();
    let _ = event_queue.push(KernelEventEnvelope::session_command(
        session_key,
        Syscall::RegisterJob {
            trigger,
            message,
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
    })
    .into())
}

// ============================================================================
// ScheduleOnceTool
// ============================================================================

pub struct ScheduleOnceTool;

impl ScheduleOnceTool {
    pub const NAME: &str = crate::tool_names::SCHEDULE_ONCE;
}

#[derive(Debug, Deserialize)]
struct ScheduleOnceParams {
    after_seconds: u64,
    message:       String,
}

#[async_trait]
impl AgentTool for ScheduleOnceTool {
    fn name(&self) -> &str { Self::NAME }

    fn description(&self) -> &str {
        "Schedule a one-shot task. It fires once after the specified delay in seconds."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["after_seconds", "message"],
            "properties": {
                "after_seconds": {
                    "type": "integer",
                    "description": "Fire once after this many seconds"
                },
                "message": {
                    "type": "string",
                    "description": "The task description to execute when the job fires"
                }
            }
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let p: ScheduleOnceParams =
            serde_json::from_value(params).map_err(|e| anyhow::anyhow!("invalid params: {e}"))?;

        if p.after_seconds == 0 {
            return Err(anyhow::anyhow!("after_seconds must be > 0"));
        }

        let now = jiff::Timestamp::now();
        let run_at = now
            .checked_add(jiff::SignedDuration::from_secs(p.after_seconds as i64))
            .map_err(|e| anyhow::anyhow!("timestamp overflow: {e}"))?;

        register_job(Trigger::Once { run_at }, p.message, context).await
    }
}

// ============================================================================
// ScheduleIntervalTool
// ============================================================================

pub struct ScheduleIntervalTool;

impl ScheduleIntervalTool {
    pub const NAME: &str = crate::tool_names::SCHEDULE_INTERVAL;
}

#[derive(Debug, Deserialize)]
struct ScheduleIntervalParams {
    interval_seconds: u64,
    message:          String,
}

#[async_trait]
impl AgentTool for ScheduleIntervalTool {
    fn name(&self) -> &str { Self::NAME }

    fn description(&self) -> &str { "Schedule a repeating task. It fires every N seconds." }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["interval_seconds", "message"],
            "properties": {
                "interval_seconds": {
                    "type": "integer",
                    "description": "Fire every N seconds (repeating)"
                },
                "message": {
                    "type": "string",
                    "description": "The task description to execute when the job fires"
                }
            }
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let p: ScheduleIntervalParams =
            serde_json::from_value(params).map_err(|e| anyhow::anyhow!("invalid params: {e}"))?;

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
            context,
        )
        .await
    }
}

// ============================================================================
// ScheduleCronTool
// ============================================================================

pub struct ScheduleCronTool;

impl ScheduleCronTool {
    pub const NAME: &str = crate::tool_names::SCHEDULE_CRON;
}

#[derive(Debug, Deserialize)]
struct ScheduleCronParams {
    cron:    String,
    message: String,
}

#[async_trait]
impl AgentTool for ScheduleCronTool {
    fn name(&self) -> &str { Self::NAME }

    fn description(&self) -> &str {
        "Schedule a task using a 6-field cron expression: 'sec min hour day month weekday' (e.g. \
         '0 0 9 * * *' for daily at 9am UTC)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["cron", "message"],
            "properties": {
                "cron": {
                    "type": "string",
                    "description": "6-field cron expression: 'sec min hour day month weekday' (e.g. '0 0 9 * * *' for daily at 9am UTC, '0 */5 * * * *' for every 5 minutes)"
                },
                "message": {
                    "type": "string",
                    "description": "The task description to execute when the job fires"
                }
            }
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let p: ScheduleCronParams =
            serde_json::from_value(params).map_err(|e| anyhow::anyhow!("invalid params: {e}"))?;

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
            context,
        )
        .await
    }
}

// ============================================================================
// ScheduleRemoveTool
// ============================================================================

/// Tool for removing a scheduled task by ID.
pub struct ScheduleRemoveTool;

impl ScheduleRemoveTool {
    pub const NAME: &str = crate::tool_names::SCHEDULE_REMOVE;
}

#[derive(Debug, Deserialize)]
struct ScheduleRemoveParams {
    job_id: String,
}

#[async_trait]
impl AgentTool for ScheduleRemoveTool {
    fn name(&self) -> &str { Self::NAME }

    fn description(&self) -> &str { "Remove a previously scheduled task by its job ID." }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["job_id"],
            "properties": {
                "job_id": {
                    "type": "string",
                    "description": "The ID of the job to remove"
                }
            }
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let p: ScheduleRemoveParams = serde_json::from_value(params)
            .map_err(|e| anyhow::anyhow!("invalid schedule-remove params: {e}"))?;

        let event_queue = context
            .event_queue
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no event queue in tool context"))?;
        let session_key = context
            .session_key
            .ok_or_else(|| anyhow::anyhow!("no session key in tool context"))?;

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

        Ok(serde_json::json!({ "ok": true }).into())
    }
}

// ============================================================================
// ScheduleListTool
// ============================================================================

/// Tool for listing all scheduled tasks in the current session.
pub struct ScheduleListTool;

impl ScheduleListTool {
    pub const NAME: &str = crate::tool_names::SCHEDULE_LIST;
}

#[async_trait]
impl AgentTool for ScheduleListTool {
    fn name(&self) -> &str { Self::NAME }

    fn description(&self) -> &str { "List all scheduled tasks for the current session." }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
        context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let event_queue = context
            .event_queue
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no event queue in tool context"))?;
        let session_key = context
            .session_key
            .ok_or_else(|| anyhow::anyhow!("no session key in tool context"))?;

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
        })
        .into())
    }
}
