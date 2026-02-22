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
use chrono::{DateTime, Utc};
use rara_domain_shared::id::SchedulerTaskId;
use sqlx::PgPool;

use crate::{
    error::SchedulerError,
    types::{ScheduledTask, TaskFilter, TaskRunRecord, TaskRunStatus},
};

// ---------------------------------------------------------------------------
// DB row types (sqlx::FromRow)
// ---------------------------------------------------------------------------

/// A scheduled task row from `scheduler_task` table.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct SchedulerTaskRow {
    pub id:            uuid::Uuid,
    pub name:          String,
    pub cron_expr:     String,
    pub enabled:       bool,
    pub last_run_at:   Option<DateTime<Utc>>,
    pub last_status:   Option<i16>,
    pub last_error:    Option<String>,
    pub run_count:     i64,
    pub failure_count: i64,
    pub is_deleted:    bool,
    pub deleted_at:    Option<DateTime<Utc>>,
    pub created_at:    DateTime<Utc>,
    pub updated_at:    DateTime<Utc>,
}

/// A task run history row from `task_run_history` table.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct TaskRunHistoryRow {
    pub id:          uuid::Uuid,
    pub task_id:     uuid::Uuid,
    pub status:      i16,
    pub started_at:  DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<i64>,
    pub error:       Option<String>,
    pub output:      Option<serde_json::Value>,
    pub created_at:  DateTime<Utc>,
}

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
        let store: SchedulerTaskRow = task.clone().into();

        let row = sqlx::query_as::<_, SchedulerTaskRow>(
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
        let row = sqlx::query_as::<_, SchedulerTaskRow>(
            "SELECT * FROM scheduler_task WHERE id = $1 AND is_deleted = FALSE",
        )
        .bind(id.into_inner())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(row.map(Into::into))
    }

    async fn find_task_by_name(&self, name: &str) -> Result<Option<ScheduledTask>, SchedulerError> {
        let row = sqlx::query_as::<_, SchedulerTaskRow>(
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

        let rows = sqlx::query_as::<_, SchedulerTaskRow>(&sql)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn update_task(&self, task: &ScheduledTask) -> Result<ScheduledTask, SchedulerError> {
        let store: SchedulerTaskRow = task.clone().into();

        let row = sqlx::query_as::<_, SchedulerTaskRow>(
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
        let store: TaskRunHistoryRow = record.clone().into();

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
        let rows = sqlx::query_as::<_, TaskRunHistoryRow>(
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
