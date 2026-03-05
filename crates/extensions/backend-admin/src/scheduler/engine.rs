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

//! Cron execution engine that loads scheduled tasks and runs them via the
//! worker framework.
//!
//! The [`CronEngine`] acts as the orchestration layer between the
//! scheduler service (persistence) and concrete [`TaskExecutor`]
//! implementations provided by other domain modules.  It:
//!
//! 1. Maintains a registry of named executors.
//! 2. Looks up enabled tasks whose names match a registered executor.
//! 3. Invokes the executor and records success/failure via
//!    [`SchedulerService::record_execution`].

use std::{collections::HashMap, sync::Arc};

use tracing::{error, info, instrument};

use super::{
    error::SchedulerError,
    service::SchedulerService,
    types::{ScheduledTask, TaskFilter, TaskRunStatus},
};

/// Trait for implementing concrete task executors.
///
/// Each scheduled task type has a corresponding executor registered
/// with the [`CronEngine`].  The executor's
/// [`task_name`](TaskExecutor::task_name) must match the
/// [`ScheduledTask::name`] stored in the database.
#[async_trait::async_trait]
pub trait TaskExecutor: Send + Sync + 'static {
    /// The task name this executor handles.
    ///
    /// Must match [`ScheduledTask::name`] for the engine to dispatch to
    /// this executor.
    fn task_name(&self) -> &str;

    /// Execute the task.
    ///
    /// Returns `Ok(())` on success. Any error is recorded against the
    /// task run history.
    async fn execute(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

/// Cron engine that manages task executor registration and dispatching.
///
/// The engine itself does **not** own a cron scheduler; it is meant to
/// be driven externally (e.g. by `raracommon-worker` `Manager` spawning
/// cron-triggered workers that call [`CronEngine::execute_task`]).
pub struct CronEngine {
    scheduler_service: Arc<SchedulerService>,
    executors:         HashMap<String, Arc<dyn TaskExecutor>>,
}

impl CronEngine {
    /// Create a new engine backed by the given scheduler service.
    pub fn new(scheduler_service: Arc<SchedulerService>) -> Self {
        Self {
            scheduler_service,
            executors: HashMap::new(),
        }
    }

    /// Register a task executor.
    ///
    /// If an executor with the same [`TaskExecutor::task_name`] already
    /// exists, it is replaced.
    pub fn register_executor(&mut self, executor: Arc<dyn TaskExecutor>) {
        let name = executor.task_name().to_string();
        info!(task_name = %name, "registered task executor");
        self.executors.insert(name, executor);
    }

    /// Return the set of registered executor names.
    pub fn registered_names(&self) -> Vec<&str> {
        self.executors.keys().map(|s| s.as_str()).collect()
    }

    /// Get all enabled tasks that have a registered executor.
    pub async fn get_runnable_tasks(&self) -> Result<Vec<ScheduledTask>, SchedulerError> {
        let tasks = self
            .scheduler_service
            .list_tasks(&TaskFilter {
                enabled: Some(true),
                ..Default::default()
            })
            .await?;

        Ok(tasks
            .into_iter()
            .filter(|t| self.executors.contains_key(&t.name))
            .collect())
    }

    /// Execute a single task by name, recording the result.
    ///
    /// The task must be enabled and have a registered executor. The
    /// execution outcome (duration, status, error message) is persisted
    /// via [`SchedulerService::record_execution`].
    #[instrument(skip(self), fields(task_name = %task_name))]
    pub async fn execute_task(&self, task_name: &str) -> Result<(), SchedulerError> {
        // Look up the task in the database.
        let tasks = self
            .scheduler_service
            .list_tasks(&TaskFilter {
                name_contains: Some(task_name.to_string()),
                enabled: Some(true),
                ..Default::default()
            })
            .await?;

        let task = tasks
            .into_iter()
            .find(|t| t.name == task_name)
            .ok_or_else(|| SchedulerError::NotFoundByName {
                name: task_name.to_string(),
            })?;

        // Look up the executor.
        let executor =
            self.executors
                .get(task_name)
                .ok_or_else(|| SchedulerError::TaskExecutionFailed {
                    task_name: task_name.to_string(),
                    message:   "no executor registered".to_string(),
                })?;

        // Run the executor, measure duration, and record the result.
        let start = std::time::Instant::now();
        match executor.execute().await {
            Ok(()) => {
                let duration_ms = start.elapsed().as_millis() as i64;
                info!(task_name, duration_ms, "task executed successfully");
                self.scheduler_service
                    .record_execution(task.id, TaskRunStatus::Success, duration_ms, None)
                    .await?;
                Ok(())
            }
            Err(e) => {
                let duration_ms = start.elapsed().as_millis() as i64;
                let error_msg = e.to_string();
                error!(task_name, duration_ms, error = %error_msg, "task execution failed");
                self.scheduler_service
                    .record_execution(
                        task.id,
                        TaskRunStatus::Failed,
                        duration_ms,
                        Some(error_msg.clone()),
                    )
                    .await?;
                Err(SchedulerError::TaskExecutionFailed {
                    task_name: task_name.to_string(),
                    message:   error_msg,
                })
            }
        }
    }
}
