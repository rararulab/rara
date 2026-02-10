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

use crate::{
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
/// be driven externally (e.g. by `job-common-worker` `Manager` spawning
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

#[cfg(test)]
mod tests {
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };

    use jiff::Timestamp;
    use job_domain_shared::id::SchedulerTaskId;

    use super::*;
    use crate::{
        repository::SchedulerRepository,
        types::{CreateTaskRequest, TaskRunRecord},
    };

    // ---- Mock executor implementations ----

    /// An executor that always succeeds.
    struct SuccessExecutor {
        name:     String,
        executed: AtomicBool,
    }

    impl SuccessExecutor {
        fn new(name: &str) -> Self {
            Self {
                name:     name.to_string(),
                executed: AtomicBool::new(false),
            }
        }

        fn was_executed(&self) -> bool { self.executed.load(Ordering::SeqCst) }
    }

    #[async_trait::async_trait]
    impl TaskExecutor for SuccessExecutor {
        fn task_name(&self) -> &str { &self.name }

        async fn execute(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            self.executed.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    /// An executor that always fails.
    struct FailingExecutor {
        name:          String,
        error_message: String,
    }

    impl FailingExecutor {
        fn new(name: &str, error: &str) -> Self {
            Self {
                name:          name.to_string(),
                error_message: error.to_string(),
            }
        }
    }

    #[async_trait::async_trait]
    impl TaskExecutor for FailingExecutor {
        fn task_name(&self) -> &str { &self.name }

        async fn execute(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            Err(self.error_message.clone().into())
        }
    }

    // ---- Mock repository (mirrors the one in service.rs) ----

    struct MockSchedulerRepo {
        tasks: Mutex<Vec<ScheduledTask>>,
        runs:  Mutex<Vec<TaskRunRecord>>,
    }

    impl MockSchedulerRepo {
        fn new() -> Self {
            Self {
                tasks: Mutex::new(Vec::new()),
                runs:  Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl SchedulerRepository for MockSchedulerRepo {
        async fn save_task(&self, task: &ScheduledTask) -> Result<ScheduledTask, SchedulerError> {
            let mut store = self.tasks.lock().unwrap();
            store.push(task.clone());
            Ok(task.clone())
        }

        async fn find_task_by_id(
            &self,
            id: SchedulerTaskId,
        ) -> Result<Option<ScheduledTask>, SchedulerError> {
            let store = self.tasks.lock().unwrap();
            Ok(store.iter().find(|t| t.id == id).cloned())
        }

        async fn find_task_by_name(
            &self,
            name: &str,
        ) -> Result<Option<ScheduledTask>, SchedulerError> {
            let store = self.tasks.lock().unwrap();
            Ok(store.iter().find(|t| t.name == name).cloned())
        }

        async fn list_tasks(
            &self,
            filter: &TaskFilter,
        ) -> Result<Vec<ScheduledTask>, SchedulerError> {
            let store = self.tasks.lock().unwrap();
            let mut results: Vec<ScheduledTask> = store.clone();
            if let Some(enabled) = filter.enabled {
                results.retain(|t| t.enabled == enabled);
            }
            if let Some(ref name_contains) = filter.name_contains {
                results.retain(|t| t.name.contains(name_contains.as_str()));
            }
            Ok(results)
        }

        async fn update_task(&self, task: &ScheduledTask) -> Result<ScheduledTask, SchedulerError> {
            let mut store = self.tasks.lock().unwrap();
            if let Some(existing) = store.iter_mut().find(|t| t.id == task.id) {
                *existing = task.clone();
                Ok(task.clone())
            } else {
                Err(SchedulerError::NotFound {
                    id: task.id.into_inner(),
                })
            }
        }

        async fn delete_task(&self, id: SchedulerTaskId) -> Result<(), SchedulerError> {
            let mut store = self.tasks.lock().unwrap();
            let len_before = store.len();
            store.retain(|t| t.id != id);
            if store.len() == len_before {
                Err(SchedulerError::NotFound {
                    id: id.into_inner(),
                })
            } else {
                Ok(())
            }
        }

        async fn record_run(&self, record: &TaskRunRecord) -> Result<(), SchedulerError> {
            let mut store = self.runs.lock().unwrap();
            store.push(record.clone());
            Ok(())
        }

        async fn get_run_history(
            &self,
            task_id: SchedulerTaskId,
            limit: i64,
        ) -> Result<Vec<TaskRunRecord>, SchedulerError> {
            let store = self.runs.lock().unwrap();
            Ok(store
                .iter()
                .filter(|r| r.task_id == task_id)
                .take(limit as usize)
                .cloned()
                .collect())
        }

        async fn update_task_last_run(
            &self,
            id: SchedulerTaskId,
            status: TaskRunStatus,
            error: Option<&str>,
        ) -> Result<(), SchedulerError> {
            let mut store = self.tasks.lock().unwrap();
            if let Some(task) = store.iter_mut().find(|t| t.id == id) {
                task.last_run_at = Some(Timestamp::now());
                task.last_status = Some(status);
                task.last_error = error.map(String::from);
                task.run_count += 1;
                if status == TaskRunStatus::Failed {
                    task.failure_count += 1;
                }
                Ok(())
            } else {
                Err(SchedulerError::NotFound {
                    id: id.into_inner(),
                })
            }
        }
    }

    // ---- Helper ----

    async fn setup_engine() -> (CronEngine, Arc<SchedulerService>) {
        let repo = Arc::new(MockSchedulerRepo::new());
        let service = Arc::new(SchedulerService::new(repo));
        let engine = CronEngine::new(Arc::clone(&service));
        (engine, service)
    }

    // ---- Tests ----

    #[tokio::test]
    async fn test_register_executor() {
        let (mut engine, _service) = setup_engine().await;

        let executor = Arc::new(SuccessExecutor::new("job-discovery"));
        engine.register_executor(executor);

        let names = engine.registered_names();
        assert_eq!(names.len(), 1);
        assert!(names.contains(&"job-discovery"));
    }

    #[tokio::test]
    async fn test_register_multiple_executors() {
        let (mut engine, _service) = setup_engine().await;

        engine.register_executor(Arc::new(SuccessExecutor::new("job-discovery")));
        engine.register_executor(Arc::new(SuccessExecutor::new("resume-refresh")));
        engine.register_executor(Arc::new(SuccessExecutor::new("metrics-snapshot")));

        let names = engine.registered_names();
        assert_eq!(names.len(), 3);
    }

    #[tokio::test]
    async fn test_register_executor_replaces_existing() {
        let (mut engine, _service) = setup_engine().await;

        engine.register_executor(Arc::new(SuccessExecutor::new("job-discovery")));
        engine.register_executor(Arc::new(SuccessExecutor::new("job-discovery")));

        let names = engine.registered_names();
        assert_eq!(names.len(), 1);
    }

    #[tokio::test]
    async fn test_execute_task_success() {
        let (mut engine, service) = setup_engine().await;

        // Register the task in the database.
        let task = service
            .register_task(CreateTaskRequest {
                name:      "job-discovery".to_string(),
                cron_expr: "0 */30 * * * *".to_string(),
            })
            .await
            .unwrap();

        // Register the executor.
        let executor = Arc::new(SuccessExecutor::new("job-discovery"));
        let executor_ref = Arc::clone(&executor);
        engine.register_executor(executor);

        // Execute.
        engine.execute_task("job-discovery").await.unwrap();

        // Verify the executor ran.
        assert!(executor_ref.was_executed());

        // Verify the execution was recorded.
        let updated = service.get_task(task.id).await.unwrap();
        assert_eq!(updated.run_count, 1);
        assert_eq!(updated.last_status, Some(TaskRunStatus::Success));
        assert!(updated.last_error.is_none());
    }

    #[tokio::test]
    async fn test_execute_task_failure_records_error() {
        let (mut engine, service) = setup_engine().await;

        let task = service
            .register_task(CreateTaskRequest {
                name:      "flaky-task".to_string(),
                cron_expr: "0 0 * * * *".to_string(),
            })
            .await
            .unwrap();

        engine.register_executor(Arc::new(FailingExecutor::new(
            "flaky-task",
            "connection refused",
        )));

        // Execute — should return an error.
        let result = engine.execute_task("flaky-task").await;
        assert!(result.is_err());

        // Verify the failure was recorded in the database.
        let updated = service.get_task(task.id).await.unwrap();
        assert_eq!(updated.run_count, 1);
        assert_eq!(updated.failure_count, 1);
        assert_eq!(updated.last_status, Some(TaskRunStatus::Failed));
        assert_eq!(updated.last_error, Some("connection refused".to_string()));

        // Verify the run history entry exists.
        let history = service.get_history(task.id, 10).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].status, TaskRunStatus::Failed);
        assert_eq!(history[0].error, Some("connection refused".to_string()));
    }

    #[tokio::test]
    async fn test_get_runnable_tasks_filters_disabled() {
        let (mut engine, service) = setup_engine().await;

        // Register two tasks.
        let _enabled_task = service
            .register_task(CreateTaskRequest {
                name:      "job-discovery".to_string(),
                cron_expr: "0 */30 * * * *".to_string(),
            })
            .await
            .unwrap();

        let disabled_task = service
            .register_task(CreateTaskRequest {
                name:      "metrics-snapshot".to_string(),
                cron_expr: "0 0 1 * * *".to_string(),
            })
            .await
            .unwrap();

        // Disable one of them.
        service.disable_task(disabled_task.id).await.unwrap();

        // Register executors for both.
        engine.register_executor(Arc::new(SuccessExecutor::new("job-discovery")));
        engine.register_executor(Arc::new(SuccessExecutor::new("metrics-snapshot")));

        // Only the enabled task should be runnable.
        let runnable = engine.get_runnable_tasks().await.unwrap();
        assert_eq!(runnable.len(), 1);
        assert_eq!(runnable[0].name, "job-discovery");
    }

    #[tokio::test]
    async fn test_get_runnable_tasks_filters_unregistered() {
        let (mut engine, service) = setup_engine().await;

        // Register two tasks in the database.
        service
            .register_task(CreateTaskRequest {
                name:      "job-discovery".to_string(),
                cron_expr: "0 */30 * * * *".to_string(),
            })
            .await
            .unwrap();

        service
            .register_task(CreateTaskRequest {
                name:      "metrics-snapshot".to_string(),
                cron_expr: "0 0 1 * * *".to_string(),
            })
            .await
            .unwrap();

        // Only register an executor for one of them.
        engine.register_executor(Arc::new(SuccessExecutor::new("job-discovery")));

        // Only the task with a registered executor should be runnable.
        let runnable = engine.get_runnable_tasks().await.unwrap();
        assert_eq!(runnable.len(), 1);
        assert_eq!(runnable[0].name, "job-discovery");
    }

    #[tokio::test]
    async fn test_execute_task_not_found() {
        let (mut engine, _service) = setup_engine().await;

        engine.register_executor(Arc::new(SuccessExecutor::new("ghost-task")));

        let result = engine.execute_task("ghost-task").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            SchedulerError::NotFoundByName { name } => {
                assert_eq!(name, "ghost-task");
            }
            other => panic!("expected NotFoundByName, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_execute_task_no_executor_registered() {
        let (engine, service) = setup_engine().await;

        // Register the task but do not register an executor.
        service
            .register_task(CreateTaskRequest {
                name:      "orphan-task".to_string(),
                cron_expr: "0 0 * * * *".to_string(),
            })
            .await
            .unwrap();

        let result = engine.execute_task("orphan-task").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            SchedulerError::TaskExecutionFailed { task_name, message } => {
                assert_eq!(task_name, "orphan-task");
                assert!(message.contains("no executor registered"));
            }
            other => panic!("expected TaskExecutionFailed, got: {other:?}"),
        }
    }
}
