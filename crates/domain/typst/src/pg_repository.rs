//! PostgreSQL-backed implementation of [`crate::repository::TypstRepository`].

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    error::{TypstError, map_db_err},
    types::{RenderResult, RenderResultRow, TypstProject, TypstProjectRow},
};

/// PostgreSQL implementation of the Typst repository.
pub struct PgTypstRepository {
    pool: PgPool,
}

impl PgTypstRepository {
    /// Create a new repository backed by the given connection pool.
    #[must_use]
    pub fn new(pool: PgPool) -> Self { Self { pool } }
}

#[async_trait]
impl crate::repository::TypstRepository for PgTypstRepository {
    // -- Projects --

    async fn create_project(
        &self,
        name: &str,
        local_path: &str,
        main_file: &str,
        git_url: Option<&str>,
    ) -> Result<TypstProject, TypstError> {
        let id = Uuid::new_v4();
        let row = sqlx::query_as::<_, TypstProjectRow>(
            r#"INSERT INTO typst_project (id, name, local_path, main_file, git_url)
               VALUES ($1, $2, $3, $4, $5)
               RETURNING *"#,
        )
        .bind(id)
        .bind(name)
        .bind(local_path)
        .bind(main_file)
        .bind(git_url)
        .fetch_one(&self.pool)
        .await
        .map_err(map_db_err)?;

        Ok(row.into())
    }

    async fn update_git_synced(&self, id: Uuid) -> Result<TypstProject, TypstError> {
        let row = sqlx::query_as::<_, TypstProjectRow>(
            r#"UPDATE typst_project SET git_last_synced_at = now()
               WHERE id = $1
               RETURNING *"#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_db_err)?
        .ok_or(TypstError::ProjectNotFound { id })?;

        Ok(row.into())
    }

    async fn get_project(&self, id: Uuid) -> Result<Option<TypstProject>, TypstError> {
        let row = sqlx::query_as::<_, TypstProjectRow>(
            "SELECT * FROM typst_project WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_db_err)?;

        Ok(row.map(Into::into))
    }

    async fn list_projects(&self) -> Result<Vec<TypstProject>, TypstError> {
        let rows = sqlx::query_as::<_, TypstProjectRow>(
            "SELECT * FROM typst_project ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_db_err)?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn delete_project(&self, id: Uuid) -> Result<(), TypstError> {
        let result =
            sqlx::query("DELETE FROM typst_project WHERE id = $1")
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(map_db_err)?;

        if result.rows_affected() == 0 {
            return Err(TypstError::ProjectNotFound { id });
        }
        Ok(())
    }

    // -- Renders --

    async fn create_render(
        &self,
        project_id: Uuid,
        pdf_object_key: &str,
        source_hash: &str,
        page_count: i32,
        file_size: i64,
    ) -> Result<RenderResult, TypstError> {
        let id = Uuid::new_v4();
        let row = sqlx::query_as::<_, RenderResultRow>(
            r#"INSERT INTO typst_render (id, project_id, pdf_object_key, source_hash, page_count, file_size)
               VALUES ($1, $2, $3, $4, $5, $6)
               RETURNING *"#,
        )
        .bind(id)
        .bind(project_id)
        .bind(pdf_object_key)
        .bind(source_hash)
        .bind(page_count)
        .bind(file_size)
        .fetch_one(&self.pool)
        .await
        .map_err(map_db_err)?;

        Ok(row.into())
    }

    async fn get_render(&self, id: Uuid) -> Result<Option<RenderResult>, TypstError> {
        let row = sqlx::query_as::<_, RenderResultRow>(
            "SELECT * FROM typst_render WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_db_err)?;

        Ok(row.map(Into::into))
    }

    async fn list_renders(&self, project_id: Uuid) -> Result<Vec<RenderResult>, TypstError> {
        let rows = sqlx::query_as::<_, RenderResultRow>(
            "SELECT * FROM typst_render WHERE project_id = $1 ORDER BY created_at DESC",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await
        .map_err(map_db_err)?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn find_render_by_hash(
        &self,
        project_id: Uuid,
        source_hash: &str,
    ) -> Result<Option<RenderResult>, TypstError> {
        let row = sqlx::query_as::<_, RenderResultRow>(
            "SELECT * FROM typst_render WHERE project_id = $1 AND source_hash = $2 ORDER BY created_at DESC LIMIT 1",
        )
        .bind(project_id)
        .bind(source_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_db_err)?;

        Ok(row.map(Into::into))
    }
}
