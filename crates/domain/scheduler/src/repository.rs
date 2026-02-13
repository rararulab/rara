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

//! Repository trait for scheduler task persistence.
//!
//! The trait lives in the domain crate so that the service layer can
//! depend on it without pulling in any infrastructure code.
//! Implementations are expected to live in the infrastructure/store
//! layer.

use rara_domain_shared::id::SchedulerTaskId;

use crate::{
    error::SchedulerError,
    types::{ScheduledTask, TaskFilter, TaskRunRecord, TaskRunStatus},
};

/// Persistence contract for scheduler tasks.
#[async_trait::async_trait]
pub trait SchedulerRepository: Send + Sync {
    /// Persist a new scheduler task.
    async fn save_task(&self, task: &ScheduledTask) -> Result<ScheduledTask, SchedulerError>;

    /// Retrieve a single task by its primary key.
    async fn find_task_by_id(
        &self,
        id: SchedulerTaskId,
    ) -> Result<Option<ScheduledTask>, SchedulerError>;

    /// Retrieve a single task by its unique name.
    async fn find_task_by_name(&self, name: &str) -> Result<Option<ScheduledTask>, SchedulerError>;

    /// List tasks matching the given filter criteria.
    async fn list_tasks(&self, filter: &TaskFilter) -> Result<Vec<ScheduledTask>, SchedulerError>;

    /// Apply updates to an existing task.
    async fn update_task(&self, task: &ScheduledTask) -> Result<ScheduledTask, SchedulerError>;

    /// Soft-delete a task by id.
    async fn delete_task(&self, id: SchedulerTaskId) -> Result<(), SchedulerError>;

    /// Record a task execution run.
    async fn record_run(&self, record: &TaskRunRecord) -> Result<(), SchedulerError>;

    /// Get the run history for a task, ordered by most recent first.
    async fn get_run_history(
        &self,
        task_id: SchedulerTaskId,
        limit: i64,
    ) -> Result<Vec<TaskRunRecord>, SchedulerError>;

    /// Update the last-run metadata on a task after execution.
    async fn update_task_last_run(
        &self,
        id: SchedulerTaskId,
        status: TaskRunStatus,
        error: Option<&str>,
    ) -> Result<(), SchedulerError>;
}
