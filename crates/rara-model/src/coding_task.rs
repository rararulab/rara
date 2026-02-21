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

//! Store model for `coding_task` table.

use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// Database row representation of a `coding_task` record.
#[derive(Debug, Clone, FromRow)]
pub struct CodingTaskRow {
    pub id:             Uuid,
    pub status:         i16,
    pub agent_type:     i16,
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
    pub created_at:     DateTime<Utc>,
    pub started_at:     Option<DateTime<Utc>>,
    pub completed_at:   Option<DateTime<Utc>>,
}
