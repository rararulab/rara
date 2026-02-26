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

//! Domain types for coding tasks.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use strum_macros::{Display, FromRepr};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Current status of a coding task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, FromRepr)]
#[repr(u8)]
pub enum CodingTaskStatus {
    Pending = 0,
    Cloning = 1,
    Running = 2,
    Completed = 3,
    Failed = 4,
    Merged = 5,
    MergeFailed = 6,
}

/// Which CLI agent to invoke.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, FromRepr)]
#[repr(u8)]
pub enum AgentType {
    Codex = 0,
    Claude = 1,
}

// ---------------------------------------------------------------------------
// Domain model
// ---------------------------------------------------------------------------

/// A persistent coding task dispatched to a CLI agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodingTask {
    pub id:             Uuid,
    pub status:         CodingTaskStatus,
    pub agent_type:     AgentType,
    pub repo_url:       String,
    pub branch:         String,
    pub prompt:         String,
    pub pr_url:         Option<String>,
    pub pr_number:      Option<i32>,
    pub session_key:    Option<String>,
    pub tmux_session:   String,
    pub workspace_path: String,
    pub output:         String,
    pub exit_code:      Option<i32>,
    pub error:          Option<String>,
    pub created_at:     Timestamp,
    pub started_at:     Option<Timestamp>,
    pub completed_at:   Option<Timestamp>,
}

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

/// Request body for dispatching a new coding task.
#[derive(Debug, Deserialize)]
pub struct CreateCodingTaskRequest {
    pub prompt:      String,
    #[serde(default = "default_agent_type")]
    pub agent_type:  AgentType,
    pub repo_url:    Option<String>,
    pub session_key: Option<String>,
}

fn default_agent_type() -> AgentType { AgentType::Claude }

/// Compact summary returned in list endpoints.
#[derive(Debug, Serialize)]
pub struct CodingTaskSummary {
    pub id:         Uuid,
    pub status:     CodingTaskStatus,
    pub agent_type: AgentType,
    pub branch:     String,
    pub prompt:     String,
    pub pr_url:     Option<String>,
    pub created_at: Timestamp,
}

/// Full detail response for a single task.
#[derive(Debug, Serialize)]
pub struct CodingTaskDetail {
    pub id:             Uuid,
    pub status:         CodingTaskStatus,
    pub agent_type:     AgentType,
    pub repo_url:       String,
    pub branch:         String,
    pub prompt:         String,
    pub pr_url:         Option<String>,
    pub pr_number:      Option<i32>,
    pub session_key:    Option<String>,
    pub tmux_session:   String,
    pub workspace_path: String,
    pub output:         String,
    pub exit_code:      Option<i32>,
    pub error:          Option<String>,
    pub created_at:     Timestamp,
    pub started_at:     Option<Timestamp>,
    pub completed_at:   Option<Timestamp>,
}

// ---------------------------------------------------------------------------
// Conversions
// ---------------------------------------------------------------------------

fn chrono_to_timestamp(dt: chrono::DateTime<chrono::Utc>) -> Timestamp {
    Timestamp::from_millisecond(dt.timestamp_millis()).unwrap_or_else(|_| Timestamp::UNIX_EPOCH)
}

fn opt_chrono_to_timestamp(dt: Option<chrono::DateTime<chrono::Utc>>) -> Option<Timestamp> {
    dt.map(chrono_to_timestamp)
}

impl From<crate::pg_repository::CodingTaskRow> for CodingTask {
    fn from(row: crate::pg_repository::CodingTaskRow) -> Self {
        Self {
            id:             row.id,
            status:         CodingTaskStatus::from_repr(row.status as u8)
                .unwrap_or(CodingTaskStatus::Pending),
            agent_type:     AgentType::from_repr(row.agent_type as u8).unwrap_or(AgentType::Claude),
            repo_url:       row.repo_url,
            branch:         row.branch,
            prompt:         row.prompt,
            pr_url:         row.pr_url,
            pr_number:      row.pr_number,
            session_key:    row.session_key,
            tmux_session:   row.tmux_session,
            workspace_path: row.workspace_path,
            output:         row.output,
            exit_code:      row.exit_code,
            error:          row.error,
            created_at:     chrono_to_timestamp(row.created_at),
            started_at:     opt_chrono_to_timestamp(row.started_at),
            completed_at:   opt_chrono_to_timestamp(row.completed_at),
        }
    }
}

impl From<&CodingTask> for CodingTaskSummary {
    fn from(task: &CodingTask) -> Self {
        Self {
            id:         task.id,
            status:     task.status,
            agent_type: task.agent_type,
            branch:     task.branch.clone(),
            prompt:     truncate(&task.prompt, 120),
            pr_url:     task.pr_url.clone(),
            created_at: task.created_at,
        }
    }
}

impl From<&CodingTask> for CodingTaskDetail {
    fn from(task: &CodingTask) -> Self {
        Self {
            id:             task.id,
            status:         task.status,
            agent_type:     task.agent_type,
            repo_url:       task.repo_url.clone(),
            branch:         task.branch.clone(),
            prompt:         task.prompt.clone(),
            pr_url:         task.pr_url.clone(),
            pr_number:      task.pr_number,
            session_key:    task.session_key.clone(),
            tmux_session:   task.tmux_session.clone(),
            workspace_path: task.workspace_path.clone(),
            output:         task.output.clone(),
            exit_code:      task.exit_code,
            error:          task.error.clone(),
            created_at:     task.created_at,
            started_at:     task.started_at,
            completed_at:   task.completed_at,
        }
    }
}

/// Truncate a string to at most `max` characters, appending "..." if
/// truncated.
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_owned()
    } else {
        let mut end = max;
        while !s.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}
