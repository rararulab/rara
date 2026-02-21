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

//! PostgreSQL implementation of [`CodingTaskRepository`].

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{CodingTaskError, NotFoundSnafu, RepositorySnafu};
use crate::repository::CodingTaskRepository;
use crate::types::{CodingTask, CodingTaskStatus};

use rara_model::coding_task::CodingTaskRow;

/// PostgreSQL-backed repository for coding tasks.
#[derive(Clone)]
pub struct PgCodingTaskRepository {
    pool: PgPool,
}

impl PgCodingTaskRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl CodingTaskRepository for PgCodingTaskRepository {
    async fn create(&self, task: &CodingTask) -> Result<CodingTask, CodingTaskError> {
        let row = sqlx::query_as::<_, CodingTaskRow>(
            r#"
            INSERT INTO coding_task (id, status, agent_type, repo_url, branch, prompt, session_key, tmux_session, workspace_path)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            RETURNING *
            "#,
        )
        .bind(task.id)
        .bind(task.status as u8 as i16)
        .bind(task.agent_type as u8 as i16)
        .bind(&task.repo_url)
        .bind(&task.branch)
        .bind(&task.prompt)
        .bind(&task.session_key)
        .bind(&task.tmux_session)
        .bind(&task.workspace_path)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| RepositorySnafu { message: e.to_string() }.build())?;
        Ok(CodingTask::from(row))
    }

    async fn get(&self, id: Uuid) -> Result<CodingTask, CodingTaskError> {
        let row = sqlx::query_as::<_, CodingTaskRow>(
            "SELECT * FROM coding_task WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositorySnafu { message: e.to_string() }.build())?
        .ok_or_else(|| NotFoundSnafu { id }.build())?;
        Ok(CodingTask::from(row))
    }

    async fn list(&self) -> Result<Vec<CodingTask>, CodingTaskError> {
        let rows = sqlx::query_as::<_, CodingTaskRow>(
            "SELECT * FROM coding_task ORDER BY created_at DESC LIMIT 100",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositorySnafu { message: e.to_string() }.build())?;
        Ok(rows.into_iter().map(CodingTask::from).collect())
    }

    async fn list_by_status(
        &self,
        status: CodingTaskStatus,
    ) -> Result<Vec<CodingTask>, CodingTaskError> {
        let rows = sqlx::query_as::<_, CodingTaskRow>(
            "SELECT * FROM coding_task WHERE status = $1 ORDER BY created_at DESC",
        )
        .bind(status as u8 as i16)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositorySnafu { message: e.to_string() }.build())?;
        Ok(rows.into_iter().map(CodingTask::from).collect())
    }

    async fn update_status(
        &self,
        id: Uuid,
        status: CodingTaskStatus,
    ) -> Result<(), CodingTaskError> {
        let result = sqlx::query(
            "UPDATE coding_task SET status = $1 WHERE id = $2",
        )
        .bind(status as u8 as i16)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositorySnafu { message: e.to_string() }.build())?;
        if result.rows_affected() == 0 {
            return Err(NotFoundSnafu { id }.build());
        }
        Ok(())
    }

    async fn update_workspace(
        &self,
        id: Uuid,
        workspace_path: &str,
        tmux_session: &str,
    ) -> Result<(), CodingTaskError> {
        sqlx::query(
            "UPDATE coding_task SET workspace_path = $1, tmux_session = $2 WHERE id = $3",
        )
        .bind(workspace_path)
        .bind(tmux_session)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositorySnafu { message: e.to_string() }.build())?;
        Ok(())
    }

    async fn update_pr(
        &self,
        id: Uuid,
        pr_url: &str,
        pr_number: i32,
    ) -> Result<(), CodingTaskError> {
        sqlx::query(
            "UPDATE coding_task SET pr_url = $1, pr_number = $2 WHERE id = $3",
        )
        .bind(pr_url)
        .bind(pr_number)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositorySnafu { message: e.to_string() }.build())?;
        Ok(())
    }

    async fn update_output(
        &self,
        id: Uuid,
        output: &str,
        exit_code: Option<i32>,
    ) -> Result<(), CodingTaskError> {
        sqlx::query(
            "UPDATE coding_task SET output = $1, exit_code = $2 WHERE id = $3",
        )
        .bind(output)
        .bind(exit_code)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositorySnafu { message: e.to_string() }.build())?;
        Ok(())
    }

    async fn update_error(&self, id: Uuid, error: &str) -> Result<(), CodingTaskError> {
        sqlx::query(
            "UPDATE coding_task SET error = $1 WHERE id = $2",
        )
        .bind(error)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositorySnafu { message: e.to_string() }.build())?;
        Ok(())
    }

    async fn set_started(&self, id: Uuid) -> Result<(), CodingTaskError> {
        sqlx::query(
            "UPDATE coding_task SET started_at = NOW() WHERE id = $1",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositorySnafu { message: e.to_string() }.build())?;
        Ok(())
    }

    async fn set_completed(&self, id: Uuid) -> Result<(), CodingTaskError> {
        sqlx::query(
            "UPDATE coding_task SET completed_at = NOW() WHERE id = $1",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositorySnafu { message: e.to_string() }.build())?;
        Ok(())
    }
}
