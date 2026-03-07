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
//! Three tools are provided:
//! - `schedule.add` — create a new scheduled job
//! - `schedule.remove` — remove a scheduled job by ID
//! - `schedule.list` — list all scheduled jobs for the current session

use async_trait::async_trait;
use serde::Deserialize;
use tokio::sync::oneshot;
use tracing::debug;

use crate::{
    event::{KernelEventEnvelope, Syscall},
    schedule::{JobId, Trigger},
    tool::{AgentTool, ToolContext},
};

// ============================================================================
// ScheduleAddTool
// ============================================================================

/// Tool for adding a scheduled task.
///
/// Accepts one of three trigger modes (mutually exclusive):
/// - `after_seconds` — fire once after N seconds
/// - `interval_seconds` — fire every N seconds
/// - `cron` — fire according to a cron expression
pub struct ScheduleAddTool;

#[derive(Debug, Deserialize)]
struct ScheduleAddParams {
    /// Fire once after this many seconds.
    #[serde(default)]
    after_seconds:    Option<u64>,
    /// Fire every N seconds (repeating).
    #[serde(default)]
    interval_seconds: Option<u64>,
    /// Cron expression (e.g. "0 9 * * *").
    #[serde(default)]
    cron:             Option<String>,
    /// The message to inject when the job fires.
    message:          String,
}

#[async_trait]
impl AgentTool for ScheduleAddTool {
    fn name(&self) -> &str { "schedule-add" }

    fn description(&self) -> &str {
        "Schedule a task to run later. Provide exactly one of: after_seconds (one-shot delay), \
         interval_seconds (repeating), or cron (cron expression). The message will be sent to the \
         current session when the job fires."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["message"],
            "properties": {
                "after_seconds": {
                    "type": "integer",
                    "description": "Fire once after this many seconds (one-shot)"
                },
                "interval_seconds": {
                    "type": "integer",
                    "description": "Fire every N seconds (repeating)"
                },
                "cron": {
                    "type": "string",
                    "description": "Cron expression (e.g. '0 9 * * *' for daily at 9am UTC)"
                },
                "message": {
                    "type": "string",
                    "description": "The message to inject when the job fires"
                }
            }
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        context: &ToolContext,
    ) -> anyhow::Result<serde_json::Value> {
        let p: ScheduleAddParams = serde_json::from_value(params)
            .map_err(|e| anyhow::anyhow!("invalid schedule-add params: {e}"))?;

        let now = jiff::Timestamp::now();

        // Treat 0 as "not provided" — LLMs often send 0 or null for unused fields.
        let after = p.after_seconds.filter(|&s| s > 0);
        let interval = p.interval_seconds.filter(|&s| s > 0);
        let cron = p.cron.filter(|s| !s.is_empty());

        let trigger = match (after, interval, cron) {
            (Some(secs), None, None) => {
                let run_at = now
                    .checked_add(jiff::SignedDuration::from_secs(secs as i64))
                    .map_err(|e| anyhow::anyhow!("timestamp overflow: {e}"))?;
                Trigger::Once { run_at }
            }
            (None, Some(secs), None) => {
                let next_at = now
                    .checked_add(jiff::SignedDuration::from_secs(secs as i64))
                    .map_err(|e| anyhow::anyhow!("timestamp overflow: {e}"))?;
                Trigger::Interval {
                    every_secs: secs,
                    next_at,
                }
            }
            (None, None, Some(expr)) => {
                // Validate the cron expression.
                use std::str::FromStr;
                let schedule = cron::Schedule::from_str(&expr)
                    .map_err(|e| anyhow::anyhow!("invalid cron expression '{expr}': {e}"))?;
                let now_chrono = chrono::DateTime::<chrono::Utc>::from_timestamp(
                    now.as_second(),
                    now.subsec_nanosecond() as u32,
                )
                .ok_or_else(|| anyhow::anyhow!("failed to convert timestamp"))?;
                let next_chrono = schedule
                    .upcoming(chrono::Utc)
                    .find(|t| *t > now_chrono)
                    .ok_or_else(|| {
                        anyhow::anyhow!("cron expression '{expr}' yields no future time")
                    })?;
                let next_at = jiff::Timestamp::from_second(next_chrono.timestamp())
                    .map_err(|e| anyhow::anyhow!("timestamp conversion error: {e}"))?;
                Trigger::Cron { expr, next_at }
            }
            _ => {
                return Err(anyhow::anyhow!(
                    "provide exactly one of: after_seconds, interval_seconds, or cron"
                ));
            }
        };

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
                message: p.message,
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
}

// ============================================================================
// ScheduleRemoveTool
// ============================================================================

/// Tool for removing a scheduled task by ID.
pub struct ScheduleRemoveTool;

#[derive(Debug, Deserialize)]
struct ScheduleRemoveParams {
    job_id: String,
}

#[async_trait]
impl AgentTool for ScheduleRemoveTool {
    fn name(&self) -> &str { "schedule-remove" }

    fn description(&self) -> &str {
        "Remove a previously scheduled task by its job ID."
    }

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
    ) -> anyhow::Result<serde_json::Value> {
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

        Ok(serde_json::json!({ "ok": true }))
    }
}

// ============================================================================
// ScheduleListTool
// ============================================================================

/// Tool for listing all scheduled tasks in the current session.
pub struct ScheduleListTool;

#[async_trait]
impl AgentTool for ScheduleListTool {
    fn name(&self) -> &str { "schedule-list" }

    fn description(&self) -> &str {
        "List all scheduled tasks for the current session."
    }

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
    ) -> anyhow::Result<serde_json::Value> {
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
        }))
    }
}
