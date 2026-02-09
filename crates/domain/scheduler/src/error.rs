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
use snafu::Snafu;
use uuid::Uuid;

/// Errors that can occur in the scheduler domain.
#[derive(Debug, Clone, Snafu, Serialize, Deserialize)]
pub enum SchedulerError {
    /// The requested scheduler task was not found.
    #[snafu(display("scheduler task not found: {id}"))]
    NotFound { id: Uuid },

    /// The requested scheduler task was not found by name.
    #[snafu(display("scheduler task not found: {name}"))]
    NotFoundByName { name: String },

    /// A storage/infrastructure error occurred.
    #[snafu(display("repository error: {message}"))]
    RepositoryError { message: String },

    /// Task execution failed.
    #[snafu(display("task '{task_name}' execution failed: {message}"))]
    TaskExecutionFailed {
        task_name: String,
        message:   String,
    },

    /// Invalid cron expression.
    #[snafu(display("invalid cron expression '{expr}': {message}"))]
    InvalidCronExpression { expr: String, message: String },

    /// Task is disabled.
    #[snafu(display("task is disabled: {id}"))]
    TaskDisabled { id: Uuid },
}
