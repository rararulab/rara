use sqlx::PgPool;
use uuid::Uuid;

use super::{
    repository::ResumeRepository,
    types::{ResumeError, ResumeProject, ResumeProjectRow},
};

pub struct PgResumeRepository {
    pool: PgPool,
}

impl PgResumeRepository {
    pub fn new(pool: PgPool) -> Self { Self { pool } }
}

#[async_trait::async_trait]
impl ResumeRepository for PgResumeRepository {
    async fn create(
        &self,
        id: Uuid,
        name: &str,
        git_url: &str,
        local_path: &str,
    ) -> Result<ResumeProject, ResumeError> {
        let row: ResumeProjectRow = sqlx::query_as(
            "INSERT INTO resume_project (id, name, git_url, local_path) VALUES ($1, $2, $3, $4) \
             RETURNING *",
        )
        .bind(id)
        .bind(name)
        .bind(git_url)
        .bind(local_path)
        .fetch_one(&self.pool)
        .await
        .map_err(|source| ResumeError::Repository { source })?;
        Ok(row.into())
    }

    async fn get(&self) -> Result<Option<ResumeProject>, ResumeError> {
        let row: Option<ResumeProjectRow> =
            sqlx::query_as("SELECT * FROM resume_project ORDER BY created_at ASC LIMIT 1")
                .fetch_optional(&self.pool)
                .await
                .map_err(|source| ResumeError::Repository { source })?;
        Ok(row.map(Into::into))
    }

    async fn get_by_id(&self, id: Uuid) -> Result<Option<ResumeProject>, ResumeError> {
        let row: Option<ResumeProjectRow> =
            sqlx::query_as("SELECT * FROM resume_project WHERE id = $1")
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|source| ResumeError::Repository { source })?;
        Ok(row.map(Into::into))
    }

    async fn update_name(&self, id: Uuid, name: &str) -> Result<ResumeProject, ResumeError> {
        let row: ResumeProjectRow =
            sqlx::query_as("UPDATE resume_project SET name = $2 WHERE id = $1 RETURNING *")
                .bind(id)
                .bind(name)
                .fetch_one(&self.pool)
                .await
                .map_err(|source| ResumeError::Repository { source })?;
        Ok(row.into())
    }

    async fn update_synced_at(&self, id: Uuid) -> Result<(), ResumeError> {
        sqlx::query("UPDATE resume_project SET last_synced_at = now() WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|source| ResumeError::Repository { source })?;
        Ok(())
    }

    async fn delete(&self, id: Uuid) -> Result<(), ResumeError> {
        sqlx::query("DELETE FROM resume_project WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|source| ResumeError::Repository { source })?;
        Ok(())
    }
}
