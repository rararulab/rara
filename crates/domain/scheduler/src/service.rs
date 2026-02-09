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

//! Scheduler service — task registration, lifecycle, and execution tracking.

use std::sync::Arc;

use chrono::Utc;
use tracing::info;
use uuid::Uuid;

use job_domain_core::id::SchedulerTaskId;

use crate::error::SchedulerError;
use crate::repository::SchedulerRepository;
use crate::types::{CreateTaskRequest, ScheduledTask, TaskFilter, TaskRunRecord, TaskRunStatus};

/// Application service for managing scheduled tasks.
pub struct SchedulerService {
    repo: Arc<dyn SchedulerRepository>,
}

impl SchedulerService {
    /// Create a new scheduler service backed by the given repository.
    pub fn new(repo: Arc<dyn SchedulerRepository>) -> Self {
        Self { repo }
    }

    /// Register a new scheduler task.
    pub async fn register_task(
        &self,
        req: CreateTaskRequest,
    ) -> Result<ScheduledTask, SchedulerError> {
        if req.name.is_empty() {
            return Err(SchedulerError::InvalidCronExpression {
                expr: req.cron_expr.clone(),
                message: "task name must not be empty".to_string(),
            });
        }

        // Validate cron expression (basic check)
        if req.cron_expr.is_empty() {
            return Err(SchedulerError::InvalidCronExpression {
                expr: req.cron_expr,
                message: "cron expression must not be empty".to_string(),
            });
        }

        // Check for duplicate name
        if self.repo.find_task_by_name(&req.name).await?.is_some() {
            return Err(SchedulerError::RepositoryError {
                message: format!("task with name '{}' already exists", req.name),
            });
        }

        let now = Utc::now();
        let task = ScheduledTask {
            id: SchedulerTaskId::new(),
            name: req.name,
            cron_expr: req.cron_expr,
            enabled: true,
            last_run_at: None,
            last_status: None,
            last_error: None,
            run_count: 0,
            failure_count: 0,
            created_at: now,
            updated_at: now,
        };

        let saved = self.repo.save_task(&task).await?;
        info!(id = %saved.id.into_inner(), name = %saved.name, "scheduler task registered");
        Ok(saved)
    }

    /// Retrieve a task by id, returning an error if not found.
    pub async fn get_task(&self, id: SchedulerTaskId) -> Result<ScheduledTask, SchedulerError> {
        self.repo
            .find_task_by_id(id)
            .await?
            .ok_or(SchedulerError::NotFound {
                id: id.into_inner(),
            })
    }

    /// Enable a previously disabled task.
    pub async fn enable_task(
        &self,
        id: SchedulerTaskId,
    ) -> Result<ScheduledTask, SchedulerError> {
        let mut task = self.get_task(id).await?;
        task.enabled = true;
        self.repo.update_task(&task).await
    }

    /// Disable a task so it will not be scheduled.
    pub async fn disable_task(
        &self,
        id: SchedulerTaskId,
    ) -> Result<ScheduledTask, SchedulerError> {
        let mut task = self.get_task(id).await?;
        task.enabled = false;
        self.repo.update_task(&task).await
    }

    /// List tasks matching the given filter.
    pub async fn list_tasks(
        &self,
        filter: &TaskFilter,
    ) -> Result<Vec<ScheduledTask>, SchedulerError> {
        self.repo.list_tasks(filter).await
    }

    /// Record a completed task execution and update the task metadata.
    pub async fn record_execution(
        &self,
        task_id: SchedulerTaskId,
        status: TaskRunStatus,
        duration_ms: i64,
        error: Option<String>,
    ) -> Result<(), SchedulerError> {
        let now = Utc::now();
        let record = TaskRunRecord {
            id: Uuid::new_v4(),
            task_id,
            status,
            started_at: now,
            finished_at: Some(now),
            duration_ms: Some(duration_ms),
            error: error.clone(),
            output: None,
            created_at: now,
        };

        self.repo.record_run(&record).await?;
        self.repo
            .update_task_last_run(task_id, status, error.as_deref())
            .await?;

        Ok(())
    }

    /// Get the execution history for a task.
    pub async fn get_history(
        &self,
        task_id: SchedulerTaskId,
        limit: i64,
    ) -> Result<Vec<TaskRunRecord>, SchedulerError> {
        self.repo.get_run_history(task_id, limit).await
    }

    /// Soft-delete a task.
    pub async fn delete_task(&self, id: SchedulerTaskId) -> Result<(), SchedulerError> {
        self.repo.delete_task(id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct MockSchedulerRepo {
        tasks: Mutex<Vec<ScheduledTask>>,
        runs: Mutex<Vec<TaskRunRecord>>,
    }

    impl MockSchedulerRepo {
        fn new() -> Self {
            Self {
                tasks: Mutex::new(Vec::new()),
                runs: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl SchedulerRepository for MockSchedulerRepo {
        async fn save_task(
            &self,
            task: &ScheduledTask,
        ) -> Result<ScheduledTask, SchedulerError> {
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

        async fn update_task(
            &self,
            task: &ScheduledTask,
        ) -> Result<ScheduledTask, SchedulerError> {
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
                task.last_run_at = Some(Utc::now());
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

    #[tokio::test]
    async fn test_register_task() {
        let repo = Arc::new(MockSchedulerRepo::new());
        let service = SchedulerService::new(repo);

        let task = service
            .register_task(CreateTaskRequest {
                name: "job-discovery".to_string(),
                cron_expr: "0 */30 * * * *".to_string(),
            })
            .await
            .unwrap();

        assert_eq!(task.name, "job-discovery");
        assert!(task.enabled);
        assert_eq!(task.run_count, 0);
    }

    #[tokio::test]
    async fn test_register_duplicate_name_fails() {
        let repo = Arc::new(MockSchedulerRepo::new());
        let service = SchedulerService::new(repo);

        service
            .register_task(CreateTaskRequest {
                name: "job-discovery".to_string(),
                cron_expr: "0 */30 * * * *".to_string(),
            })
            .await
            .unwrap();

        let result = service
            .register_task(CreateTaskRequest {
                name: "job-discovery".to_string(),
                cron_expr: "0 */15 * * * *".to_string(),
            })
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_enable_disable_task() {
        let repo = Arc::new(MockSchedulerRepo::new());
        let service = SchedulerService::new(repo);

        let task = service
            .register_task(CreateTaskRequest {
                name: "test-task".to_string(),
                cron_expr: "0 0 * * * *".to_string(),
            })
            .await
            .unwrap();

        let disabled = service.disable_task(task.id).await.unwrap();
        assert!(!disabled.enabled);

        let enabled = service.enable_task(task.id).await.unwrap();
        assert!(enabled.enabled);
    }

    #[tokio::test]
    async fn test_record_execution() {
        let repo = Arc::new(MockSchedulerRepo::new());
        let service = SchedulerService::new(repo);

        let task = service
            .register_task(CreateTaskRequest {
                name: "test-task".to_string(),
                cron_expr: "0 0 * * * *".to_string(),
            })
            .await
            .unwrap();

        service
            .record_execution(task.id, TaskRunStatus::Success, 1500, None)
            .await
            .unwrap();

        let updated = service.get_task(task.id).await.unwrap();
        assert_eq!(updated.run_count, 1);
        assert!(updated.last_run_at.is_some());
        assert_eq!(updated.last_status, Some(TaskRunStatus::Success));
    }

    #[tokio::test]
    async fn test_record_failed_execution_increments_failure_count() {
        let repo = Arc::new(MockSchedulerRepo::new());
        let service = SchedulerService::new(repo);

        let task = service
            .register_task(CreateTaskRequest {
                name: "test-task".to_string(),
                cron_expr: "0 0 * * * *".to_string(),
            })
            .await
            .unwrap();

        service
            .record_execution(
                task.id,
                TaskRunStatus::Failed,
                500,
                Some("connection refused".to_string()),
            )
            .await
            .unwrap();

        let updated = service.get_task(task.id).await.unwrap();
        assert_eq!(updated.failure_count, 1);
        assert_eq!(updated.last_error, Some("connection refused".to_string()));
    }

    #[tokio::test]
    async fn test_get_run_history() {
        let repo = Arc::new(MockSchedulerRepo::new());
        let service = SchedulerService::new(repo);

        let task = service
            .register_task(CreateTaskRequest {
                name: "test-task".to_string(),
                cron_expr: "0 0 * * * *".to_string(),
            })
            .await
            .unwrap();

        service
            .record_execution(task.id, TaskRunStatus::Success, 1000, None)
            .await
            .unwrap();
        service
            .record_execution(
                task.id,
                TaskRunStatus::Failed,
                500,
                Some("error".to_string()),
            )
            .await
            .unwrap();

        let history = service.get_history(task.id, 10).await.unwrap();
        assert_eq!(history.len(), 2);
    }

    #[tokio::test]
    async fn test_list_tasks_with_filter() {
        let repo = Arc::new(MockSchedulerRepo::new());
        let service = SchedulerService::new(repo);

        service
            .register_task(CreateTaskRequest {
                name: "job-discovery".to_string(),
                cron_expr: "0 */30 * * * *".to_string(),
            })
            .await
            .unwrap();

        let task2 = service
            .register_task(CreateTaskRequest {
                name: "metrics-snapshot".to_string(),
                cron_expr: "0 0 1 * * *".to_string(),
            })
            .await
            .unwrap();

        service.disable_task(task2.id).await.unwrap();

        let enabled_only = service
            .list_tasks(&TaskFilter {
                enabled: Some(true),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(enabled_only.len(), 1);
        assert_eq!(enabled_only[0].name, "job-discovery");
    }

    #[tokio::test]
    async fn test_delete_task() {
        let repo = Arc::new(MockSchedulerRepo::new());
        let service = SchedulerService::new(repo);

        let task = service
            .register_task(CreateTaskRequest {
                name: "temp-task".to_string(),
                cron_expr: "0 0 * * * *".to_string(),
            })
            .await
            .unwrap();

        service.delete_task(task.id).await.unwrap();
        let result = service.get_task(task.id).await;
        assert!(result.is_err());
    }
}
