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

//! PostgreSQL-backed implementation of
//! [`crate::repository::SchedulerRepository`].

use std::fmt::Write;

use async_trait::async_trait;
use job_domain_shared::id::SchedulerTaskId;
use sqlx::PgPool;

use crate::{
    db_models,
    error::SchedulerError,
    types::{ScheduledTask, TaskFilter, TaskRunRecord, TaskRunStatus},
};

/// PostgreSQL implementation of the scheduler repository.
pub struct PgSchedulerRepository {
    pool: PgPool,
}

impl PgSchedulerRepository {
    /// Create a new repository backed by the given connection pool.
    #[must_use]
    pub fn new(pool: PgPool) -> Self { Self { pool } }
}

/// Map a `sqlx::Error` into a `SchedulerError::RepositoryError`.
fn map_err(e: sqlx::Error) -> SchedulerError {
    SchedulerError::RepositoryError {
        message: e.to_string(),
    }
}

#[async_trait]
impl crate::repository::SchedulerRepository for PgSchedulerRepository {
    async fn save_task(&self, task: &ScheduledTask) -> Result<ScheduledTask, SchedulerError> {
        let store: db_models::SchedulerTask = task.clone().into();

        let row = sqlx::query_as::<_, db_models::SchedulerTask>(
            r#"INSERT INTO scheduler_task
                   (id, name, cron_expr, enabled, last_run_at, last_status,
                    last_error, run_count, failure_count, is_deleted, created_at, updated_at)
               VALUES
                   ($1, $2, $3, $4, $5, $6,
                    $7, $8, $9, $10, $11, $12)
               RETURNING *"#,
        )
        .bind(store.id)
        .bind(&store.name)
        .bind(&store.cron_expr)
        .bind(store.enabled)
        .bind(store.last_run_at)
        .bind(&store.last_status)
        .bind(&store.last_error)
        .bind(store.run_count)
        .bind(store.failure_count)
        .bind(store.is_deleted)
        .bind(store.created_at)
        .bind(store.updated_at)
        .fetch_one(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(row.into())
    }

    async fn find_task_by_id(
        &self,
        id: SchedulerTaskId,
    ) -> Result<Option<ScheduledTask>, SchedulerError> {
        let row = sqlx::query_as::<_, db_models::SchedulerTask>(
            "SELECT * FROM scheduler_task WHERE id = $1 AND is_deleted = FALSE",
        )
        .bind(id.into_inner())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(row.map(Into::into))
    }

    async fn find_task_by_name(&self, name: &str) -> Result<Option<ScheduledTask>, SchedulerError> {
        let row = sqlx::query_as::<_, db_models::SchedulerTask>(
            "SELECT * FROM scheduler_task WHERE name = $1 AND is_deleted = FALSE",
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(row.map(Into::into))
    }

    async fn list_tasks(&self, filter: &TaskFilter) -> Result<Vec<ScheduledTask>, SchedulerError> {
        let mut sql = String::from("SELECT * FROM scheduler_task WHERE is_deleted = FALSE");

        if let Some(enabled) = filter.enabled {
            let _ = write!(sql, " AND enabled = {enabled}");
        }

        if let Some(ref name_contains) = filter.name_contains {
            let escaped = name_contains.replace('\'', "''");
            let _ = write!(sql, " AND name LIKE '%{escaped}%'");
        }

        sql.push_str(" ORDER BY created_at DESC");

        let rows = sqlx::query_as::<_, db_models::SchedulerTask>(&sql)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn update_task(&self, task: &ScheduledTask) -> Result<ScheduledTask, SchedulerError> {
        let store: db_models::SchedulerTask = task.clone().into();

        let row = sqlx::query_as::<_, db_models::SchedulerTask>(
            r#"UPDATE scheduler_task
               SET name = $2, cron_expr = $3, enabled = $4, last_run_at = $5,
                   last_status = $6, last_error = $7,
                   run_count = $8, failure_count = $9
               WHERE id = $1 AND is_deleted = FALSE
               RETURNING *"#,
        )
        .bind(store.id)
        .bind(&store.name)
        .bind(&store.cron_expr)
        .bind(store.enabled)
        .bind(store.last_run_at)
        .bind(&store.last_status)
        .bind(&store.last_error)
        .bind(store.run_count)
        .bind(store.failure_count)
        .fetch_one(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(row.into())
    }

    async fn delete_task(&self, id: SchedulerTaskId) -> Result<(), SchedulerError> {
        let result = sqlx::query(
            "UPDATE scheduler_task SET is_deleted = TRUE, deleted_at = now() WHERE id = $1 AND \
             is_deleted = FALSE",
        )
        .bind(id.into_inner())
        .execute(&self.pool)
        .await
        .map_err(map_err)?;

        if result.rows_affected() == 0 {
            return Err(SchedulerError::NotFound {
                id: id.into_inner(),
            });
        }
        Ok(())
    }

    async fn record_run(&self, record: &TaskRunRecord) -> Result<(), SchedulerError> {
        let store: db_models::TaskRunHistory = record.clone().into();

        sqlx::query(
            r#"INSERT INTO task_run_history
                   (id, task_id, status, started_at, finished_at, duration_ms, error, output, created_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)"#,
        )
        .bind(store.id)
        .bind(store.task_id)
        .bind(&store.status)
        .bind(store.started_at)
        .bind(store.finished_at)
        .bind(store.duration_ms)
        .bind(&store.error)
        .bind(&store.output)
        .bind(store.created_at)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(())
    }

    async fn get_run_history(
        &self,
        task_id: SchedulerTaskId,
        limit: i64,
    ) -> Result<Vec<TaskRunRecord>, SchedulerError> {
        let rows = sqlx::query_as::<_, db_models::TaskRunHistory>(
            r#"SELECT * FROM task_run_history
               WHERE task_id = $1
               ORDER BY started_at DESC
               LIMIT $2"#,
        )
        .bind(task_id.into_inner())
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn update_task_last_run(
        &self,
        id: SchedulerTaskId,
        status: TaskRunStatus,
        error: Option<&str>,
    ) -> Result<(), SchedulerError> {
        let status_code = status as u8 as i16;

        let sql = if status == TaskRunStatus::Failed {
            r#"UPDATE scheduler_task
               SET last_run_at = now(), last_status = $2, last_error = $3,
                   run_count = run_count + 1, failure_count = failure_count + 1
               WHERE id = $1 AND is_deleted = FALSE"#
        } else {
            r#"UPDATE scheduler_task
               SET last_run_at = now(), last_status = $2, last_error = $3,
                   run_count = run_count + 1
               WHERE id = $1 AND is_deleted = FALSE"#
        };

        let result = sqlx::query(sql)
            .bind(id.into_inner())
            .bind(status_code)
            .bind(error)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;

        if result.rows_affected() == 0 {
            return Err(SchedulerError::NotFound {
                id: id.into_inner(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use jiff::Timestamp;
    use job_domain_shared::id::SchedulerTaskId;
    use sqlx::postgres::PgPoolOptions;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;
    use uuid::Uuid;

    use super::*;
    use crate::repository::SchedulerRepository;

    async fn connect_pool(url: &str) -> sqlx::PgPool {
        let mut last_err: Option<sqlx::Error> = None;
        for _ in 0..30 {
            match PgPoolOptions::new()
                .max_connections(5)
                .acquire_timeout(Duration::from_secs(10))
                .connect(url)
                .await
            {
                Ok(pool) => return pool,
                Err(e) => {
                    last_err = Some(e);
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
        }
        panic!("failed to connect to postgres: {last_err:?}");
    }

    async fn setup_pool() -> (sqlx::PgPool, testcontainers::ContainerAsync<Postgres>) {
        let container = Postgres::default().start().await.unwrap();
        let host = container.get_host().await.unwrap();
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
        let pool = connect_pool(&url).await;

        // Ensure gen_random_uuid() is available (older PG images need pgcrypto).
        sqlx::raw_sql("CREATE EXTENSION IF NOT EXISTS \"pgcrypto\"")
            .execute(&pool)
            .await
            .unwrap();

        // Run all migrations in order using raw_sql (simple query protocol)
        // which supports multi-statement execution.
        let migrations: &[&str] = &[
            include_str!("../../../job-model/migrations/20260127000000_init.sql"),
            include_str!("../../../job-model/migrations/20260208000000_domain_models.sql"),
            include_str!("../../../job-model/migrations/20260209000000_resume_version_mgmt.sql"),
            include_str!("../../../job-model/migrations/20260210000000_schema_alignment.sql"),
            include_str!("../../../job-model/migrations/20260211000000_notify_priority.sql"),
        ];

        for sql in migrations {
            sqlx::raw_sql(sql).execute(&pool).await.unwrap();
        }

        // The scheduler migration references set_updated_at() but the
        // function was created as trigger_set_updated_at() in the domain
        // migration. Fix the reference before executing.
        let scheduler_sql =
            include_str!("../../../job-model/migrations/20260211000001_scheduler_tables.sql")
                .replace(
                    "FUNCTION set_updated_at()",
                    "FUNCTION trigger_set_updated_at()",
                );
        sqlx::raw_sql(&scheduler_sql).execute(&pool).await.unwrap();

        // Convert domain enum columns to SMALLINT codes.
        let domain_int_migrations: &[&str] = &[
            include_str!("../../../job-model/migrations/20260212000000_application_int_enums.sql"),
            include_str!("../../../job-model/migrations/20260212000001_interview_int_enums.sql"),
            include_str!("../../../job-model/migrations/20260212000002_notify_int_enums.sql"),
            include_str!("../../../job-model/migrations/20260212000003_resume_int_enums.sql"),
            include_str!("../../../job-model/migrations/20260212000004_scheduler_int_enums.sql"),
        ];
        for sql in domain_int_migrations {
            sqlx::raw_sql(sql).execute(&pool).await.unwrap();
        }

        (pool, container)
    }

    fn make_task() -> ScheduledTask {
        let now = Timestamp::now();
        ScheduledTask {
            id:            SchedulerTaskId::new(),
            name:          format!("test-task-{}", Uuid::new_v4()),
            cron_expr:     "0 */30 * * * *".into(),
            enabled:       true,
            last_run_at:   None,
            last_status:   None,
            last_error:    None,
            run_count:     0,
            failure_count: 0,
            created_at:    now,
            updated_at:    now,
        }
    }

    fn make_run_record(task_id: SchedulerTaskId) -> TaskRunRecord {
        let now = Timestamp::now();
        TaskRunRecord {
            id: Uuid::new_v4(),
            task_id,
            status: TaskRunStatus::Success,
            started_at: now,
            finished_at: Some(now),
            duration_ms: Some(1500),
            error: None,
            output: None,
            created_at: now,
        }
    }

    #[tokio::test]
    async fn test_save_and_find_by_id() {
        let (pool, _container) = setup_pool().await;
        let repo = PgSchedulerRepository::new(pool);

        let task = make_task();
        let saved = repo.save_task(&task).await.unwrap();
        assert_eq!(saved.id, task.id);
        assert_eq!(saved.name, task.name);
        assert!(saved.enabled);

        let found = repo.find_task_by_id(saved.id).await.unwrap();
        assert!(found.is_some());
        let found = found.unwrap();
        assert_eq!(found.id, saved.id);
        assert_eq!(found.cron_expr, "0 */30 * * * *");
    }

    #[tokio::test]
    async fn test_find_by_id_not_found() {
        let (pool, _container) = setup_pool().await;
        let repo = PgSchedulerRepository::new(pool);

        let found = repo.find_task_by_id(SchedulerTaskId::new()).await.unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn test_find_by_name() {
        let (pool, _container) = setup_pool().await;
        let repo = PgSchedulerRepository::new(pool);

        let mut task = make_task();
        task.name = "unique-task-name".into();
        repo.save_task(&task).await.unwrap();

        let found = repo.find_task_by_name("unique-task-name").await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "unique-task-name");

        let not_found = repo.find_task_by_name("nonexistent").await.unwrap();
        assert!(not_found.is_none());
    }

    #[tokio::test]
    async fn test_list_tasks_with_filter() {
        let (pool, _container) = setup_pool().await;
        let repo = PgSchedulerRepository::new(pool);

        let mut t1 = make_task();
        t1.name = "job-discovery".into();
        t1.enabled = true;
        repo.save_task(&t1).await.unwrap();

        let mut t2 = make_task();
        t2.name = "metrics-snapshot".into();
        t2.enabled = false;
        repo.save_task(&t2).await.unwrap();

        // Filter enabled only
        let filter = TaskFilter {
            enabled: Some(true),
            ..Default::default()
        };
        let results = repo.list_tasks(&filter).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "job-discovery");

        // Filter by name contains
        let filter = TaskFilter {
            name_contains: Some("metrics".into()),
            ..Default::default()
        };
        let results = repo.list_tasks(&filter).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "metrics-snapshot");

        // No filter - get all
        let all = repo.list_tasks(&TaskFilter::default()).await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn test_update_task() {
        let (pool, _container) = setup_pool().await;
        let repo = PgSchedulerRepository::new(pool);

        let task = make_task();
        let saved = repo.save_task(&task).await.unwrap();

        let mut updated = saved.clone();
        updated.cron_expr = "0 0 * * * *".into();
        updated.enabled = false;

        let result = repo.update_task(&updated).await.unwrap();
        assert_eq!(result.cron_expr, "0 0 * * * *");
        assert!(!result.enabled);
    }

    #[tokio::test]
    async fn test_delete_task() {
        let (pool, _container) = setup_pool().await;
        let repo = PgSchedulerRepository::new(pool);

        let task = make_task();
        let saved = repo.save_task(&task).await.unwrap();

        repo.delete_task(saved.id).await.unwrap();

        // Soft-deleted should not be found
        let found = repo.find_task_by_id(saved.id).await.unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn test_delete_task_not_found() {
        let (pool, _container) = setup_pool().await;
        let repo = PgSchedulerRepository::new(pool);

        let result = repo.delete_task(SchedulerTaskId::new()).await;
        assert!(matches!(result, Err(SchedulerError::NotFound { .. })));
    }

    #[tokio::test]
    async fn test_record_run_and_get_history() {
        let (pool, _container) = setup_pool().await;
        let repo = PgSchedulerRepository::new(pool);

        let task = make_task();
        let saved = repo.save_task(&task).await.unwrap();

        let record1 = make_run_record(saved.id);
        repo.record_run(&record1).await.unwrap();

        let mut record2 = make_run_record(saved.id);
        record2.status = TaskRunStatus::Failed;
        record2.error = Some("connection refused".into());
        repo.record_run(&record2).await.unwrap();

        let history = repo.get_run_history(saved.id, 10).await.unwrap();
        assert_eq!(history.len(), 2);
    }

    #[tokio::test]
    async fn test_get_run_history_respects_limit() {
        let (pool, _container) = setup_pool().await;
        let repo = PgSchedulerRepository::new(pool);

        let task = make_task();
        let saved = repo.save_task(&task).await.unwrap();

        for _ in 0..5 {
            let record = make_run_record(saved.id);
            repo.record_run(&record).await.unwrap();
        }

        let history = repo.get_run_history(saved.id, 3).await.unwrap();
        assert_eq!(history.len(), 3);
    }

    #[tokio::test]
    async fn test_update_task_last_run_success() {
        let (pool, _container) = setup_pool().await;
        let repo = PgSchedulerRepository::new(pool);

        let task = make_task();
        let saved = repo.save_task(&task).await.unwrap();
        assert_eq!(saved.run_count, 0);

        repo.update_task_last_run(saved.id, TaskRunStatus::Success, None)
            .await
            .unwrap();

        let updated = repo.find_task_by_id(saved.id).await.unwrap().unwrap();
        assert_eq!(updated.run_count, 1);
        assert_eq!(updated.failure_count, 0);
        assert!(updated.last_run_at.is_some());
        assert_eq!(updated.last_status, Some(TaskRunStatus::Success));
    }

    #[tokio::test]
    async fn test_update_task_last_run_failed() {
        let (pool, _container) = setup_pool().await;
        let repo = PgSchedulerRepository::new(pool);

        let task = make_task();
        let saved = repo.save_task(&task).await.unwrap();

        repo.update_task_last_run(saved.id, TaskRunStatus::Failed, Some("connection refused"))
            .await
            .unwrap();

        let updated = repo.find_task_by_id(saved.id).await.unwrap().unwrap();
        assert_eq!(updated.run_count, 1);
        assert_eq!(updated.failure_count, 1);
        assert_eq!(updated.last_error, Some("connection refused".into()));
        assert_eq!(updated.last_status, Some(TaskRunStatus::Failed));
    }

    #[tokio::test]
    async fn test_update_task_last_run_not_found() {
        let (pool, _container) = setup_pool().await;
        let repo = PgSchedulerRepository::new(pool);

        let result = repo
            .update_task_last_run(SchedulerTaskId::new(), TaskRunStatus::Success, None)
            .await;
        assert!(matches!(result, Err(SchedulerError::NotFound { .. })));
    }

    #[tokio::test]
    async fn test_toggle_enabled() {
        let (pool, _container) = setup_pool().await;
        let repo = PgSchedulerRepository::new(pool);

        let task = make_task();
        let saved = repo.save_task(&task).await.unwrap();
        assert!(saved.enabled);

        // Disable
        let mut disabled = saved.clone();
        disabled.enabled = false;
        let updated = repo.update_task(&disabled).await.unwrap();
        assert!(!updated.enabled);

        // Re-enable
        let mut enabled = updated.clone();
        enabled.enabled = true;
        let updated = repo.update_task(&enabled).await.unwrap();
        assert!(updated.enabled);
    }
}
