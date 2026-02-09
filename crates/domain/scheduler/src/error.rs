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

//! Error types for the scheduler domain.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Errors that can occur in the scheduler domain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SchedulerError {
    /// The requested scheduler task was not found.
    NotFound { id: Uuid },
    /// The requested scheduler task was not found by name.
    NotFoundByName { name: String },
    /// A storage/infrastructure error occurred.
    RepositoryError { message: String },
    /// Task execution failed.
    TaskExecutionFailed { task_name: String, message: String },
    /// Invalid cron expression.
    InvalidCronExpression { expr: String, message: String },
    /// Task is disabled.
    TaskDisabled { id: Uuid },
}

impl std::fmt::Display for SchedulerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound { id } => write!(f, "scheduler task not found: {id}"),
            Self::NotFoundByName { name } => write!(f, "scheduler task not found: {name}"),
            Self::RepositoryError { message } => write!(f, "repository error: {message}"),
            Self::TaskExecutionFailed { task_name, message } => {
                write!(f, "task '{task_name}' execution failed: {message}")
            }
            Self::InvalidCronExpression { expr, message } => {
                write!(f, "invalid cron expression '{expr}': {message}")
            }
            Self::TaskDisabled { id } => write!(f, "task is disabled: {id}"),
        }
    }
}

impl std::error::Error for SchedulerError {}
