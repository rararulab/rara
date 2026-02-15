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

//! Layer 2 service tools for managing the agent scheduler.

use std::sync::Arc;

use async_trait::async_trait;
use rara_agents::tool_registry::AgentTool;
use serde_json::json;

use std::str::FromStr;

use crate::agent_scheduler::{AgentJob, AgentScheduler, AgentTrigger};

// ---------------------------------------------------------------------------
// ScheduleAddTool
// ---------------------------------------------------------------------------

/// Tool that lets the agent schedule a new recurring or one-shot job.
pub struct ScheduleAddTool {
    scheduler: Arc<AgentScheduler>,
}

impl ScheduleAddTool {
    pub fn new(scheduler: Arc<AgentScheduler>) -> Self {
        Self { scheduler }
    }
}

#[async_trait]
impl AgentTool for ScheduleAddTool {
    fn name(&self) -> &str {
        "schedule_add"
    }

    fn description(&self) -> &str {
        "Schedule a new agent job. The message will be executed as a user prompt when the trigger fires."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "Natural-language intent to execute when the job fires"
                },
                "trigger_type": {
                    "type": "string",
                    "enum": ["cron", "delay", "interval"],
                    "description": "Trigger type: cron (recurring via cron expr), delay (one-shot after N seconds), interval (recurring every N seconds)"
                },
                "cron_expr": {
                    "type": "string",
                    "description": "Cron expression (5-field format). Required when trigger_type=cron"
                },
                "delay_seconds": {
                    "type": "integer",
                    "description": "Delay in seconds from now. Required when trigger_type=delay"
                },
                "interval_seconds": {
                    "type": "integer",
                    "description": "Interval in seconds. Required when trigger_type=interval"
                }
            },
            "required": ["message", "trigger_type"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
    ) -> rara_agents::err::Result<serde_json::Value> {
        let message = params
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| rara_agents::err::Error::Other {
                message: "missing required parameter: message".into(),
            })?
            .to_owned();

        let trigger_type = params
            .get("trigger_type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| rara_agents::err::Error::Other {
                message: "missing required parameter: trigger_type".into(),
            })?;

        let trigger = match trigger_type {
            "cron" => {
                let expr = params
                    .get("cron_expr")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| rara_agents::err::Error::Other {
                        message: "cron_expr is required when trigger_type=cron".into(),
                    })?
                    .to_owned();
                // Validate the expression.
                croner::Cron::from_str(&expr).map_err(|e| rara_agents::err::Error::Other {
                    message: format!("invalid cron expression: {e}").into(),
                })?;
                AgentTrigger::Cron { expr }
            }
            "delay" => {
                let seconds = params
                    .get("delay_seconds")
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| rara_agents::err::Error::Other {
                        message: "delay_seconds is required when trigger_type=delay".into(),
                    })?;
                let run_at = jiff::Timestamp::now() + std::time::Duration::from_secs(seconds);
                AgentTrigger::Delay { run_at }
            }
            "interval" => {
                let seconds = params
                    .get("interval_seconds")
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| rara_agents::err::Error::Other {
                        message: "interval_seconds is required when trigger_type=interval".into(),
                    })?;
                AgentTrigger::Interval { seconds }
            }
            other => {
                return Err(rara_agents::err::Error::Other {
                    message: format!("unknown trigger_type: {other}").into(),
                });
            }
        };

        let id = ulid::Ulid::new().to_string();
        let job = AgentJob {
            id:          id.clone(),
            message,
            trigger,
            session_key: "agent:proactive".to_owned(),
            created_at:  jiff::Timestamp::now(),
            last_run_at: None,
            enabled:     true,
        };

        self.scheduler
            .add(job)
            .await
            .map_err(|e| rara_agents::err::Error::Other {
                message: format!("failed to add job: {e}").into(),
            })?;

        Ok(json!({ "status": "ok", "id": id }))
    }
}

// ---------------------------------------------------------------------------
// ScheduleListTool
// ---------------------------------------------------------------------------

/// Tool that lists all scheduled agent jobs.
pub struct ScheduleListTool {
    scheduler: Arc<AgentScheduler>,
}

impl ScheduleListTool {
    pub fn new(scheduler: Arc<AgentScheduler>) -> Self {
        Self { scheduler }
    }
}

#[async_trait]
impl AgentTool for ScheduleListTool {
    fn name(&self) -> &str {
        "schedule_list"
    }

    fn description(&self) -> &str {
        "List all scheduled agent jobs with their triggers and status."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
    ) -> rara_agents::err::Result<serde_json::Value> {
        let jobs = self.scheduler.list().await;
        let items: Vec<serde_json::Value> = jobs
            .iter()
            .map(|j| {
                json!({
                    "id": j.id,
                    "message": j.message,
                    "trigger": j.trigger,
                    "session_key": j.session_key,
                    "created_at": j.created_at.to_string(),
                    "last_run_at": j.last_run_at.map(|t| t.to_string()),
                    "enabled": j.enabled,
                })
            })
            .collect();
        Ok(json!({ "count": items.len(), "jobs": items }))
    }
}

// ---------------------------------------------------------------------------
// ScheduleRemoveTool
// ---------------------------------------------------------------------------

/// Tool that removes a scheduled agent job by ID.
pub struct ScheduleRemoveTool {
    scheduler: Arc<AgentScheduler>,
}

impl ScheduleRemoveTool {
    pub fn new(scheduler: Arc<AgentScheduler>) -> Self {
        Self { scheduler }
    }
}

#[async_trait]
impl AgentTool for ScheduleRemoveTool {
    fn name(&self) -> &str {
        "schedule_remove"
    }

    fn description(&self) -> &str {
        "Remove a scheduled agent job by its ID."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "The job ID to remove"
                }
            },
            "required": ["id"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
    ) -> rara_agents::err::Result<serde_json::Value> {
        let id = params
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| rara_agents::err::Error::Other {
                message: "missing required parameter: id".into(),
            })?;

        let removed = self
            .scheduler
            .remove(id)
            .await
            .map_err(|e| rara_agents::err::Error::Other {
                message: format!("failed to remove job: {e}").into(),
            })?;

        if removed {
            Ok(json!({ "status": "ok", "removed": true }))
        } else {
            Ok(json!({ "status": "not_found", "removed": false }))
        }
    }
}
