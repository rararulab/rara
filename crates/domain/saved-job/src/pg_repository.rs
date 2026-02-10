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

//! PostgreSQL-backed implementation of [`crate::repository::SavedJobRepository`].

use async_trait::async_trait;
use jiff::Timestamp;
use job_domain_shared::convert::timestamp_to_chrono;
use job_model::saved_job::SavedJob as StoreSavedJob;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::SavedJobError;
use crate::types::{SavedJob, SavedJobStatus};

/// PostgreSQL implementation of the saved-job repository.
pub struct PgSavedJobRepository {
    pool: PgPool,
}

impl PgSavedJobRepository {
    /// Create a new repository backed by the given connection pool.
    #[must_use]
    pub fn new(pool: PgPool) -> Self { Self { pool } }
}

/// Map a `sqlx::Error` into a `SavedJobError::RepositoryError`.
fn map_err(e: sqlx::Error) -> SavedJobError {
    SavedJobError::RepositoryError {
        message: e.to_string(),
    }
}

#[async_trait]
impl crate::repository::SavedJobRepository for PgSavedJobRepository {
    async fn create(&self, url: &str) -> Result<SavedJob, SavedJobError> {
        let row = sqlx::query_as::<_, StoreSavedJob>(
            r#"INSERT INTO saved_job (url, status)
               VALUES ($1, $2)
               RETURNING *"#,
        )
        .bind(url)
        .bind(SavedJobStatus::PendingCrawl as u8 as i16)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            // Check for unique constraint violation on url
            if let sqlx::Error::Database(ref db_err) = e {
                if db_err.constraint() == Some("saved_job_url_key") {
                    return SavedJobError::DuplicateUrl {
                        url: url.to_owned(),
                    };
                }
            }
            map_err(e)
        })?;

        Ok(row.into())
    }

    async fn get_by_id(&self, id: Uuid) -> Result<Option<SavedJob>, SavedJobError> {
        let row = sqlx::query_as::<_, StoreSavedJob>("SELECT * FROM saved_job WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err)?;

        Ok(row.map(Into::into))
    }

    async fn list(&self, status: Option<SavedJobStatus>) -> Result<Vec<SavedJob>, SavedJobError> {
        let rows = if let Some(status) = status {
            sqlx::query_as::<_, StoreSavedJob>(
                "SELECT * FROM saved_job WHERE status = $1 ORDER BY created_at DESC",
            )
            .bind(status as u8 as i16)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?
        } else {
            sqlx::query_as::<_, StoreSavedJob>(
                "SELECT * FROM saved_job ORDER BY created_at DESC",
            )
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?
        };

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn delete(&self, id: Uuid) -> Result<(), SavedJobError> {
        let result = sqlx::query("DELETE FROM saved_job WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;

        if result.rows_affected() == 0 {
            return Err(SavedJobError::NotFound { id });
        }
        Ok(())
    }

    async fn update_status(
        &self,
        id: Uuid,
        status: SavedJobStatus,
        error_message: Option<String>,
    ) -> Result<(), SavedJobError> {
        let result = sqlx::query(
            "UPDATE saved_job SET status = $2, error_message = $3 WHERE id = $1",
        )
        .bind(id)
        .bind(status as u8 as i16)
        .bind(error_message)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;

        if result.rows_affected() == 0 {
            return Err(SavedJobError::NotFound { id });
        }
        Ok(())
    }

    async fn update_crawl_result(
        &self,
        id: Uuid,
        s3_key: &str,
        preview: &str,
    ) -> Result<(), SavedJobError> {
        let result = sqlx::query(
            r#"UPDATE saved_job
               SET markdown_s3_key = $2, markdown_preview = $3,
                   status = $4, crawled_at = now()
               WHERE id = $1"#,
        )
        .bind(id)
        .bind(s3_key)
        .bind(preview)
        .bind(SavedJobStatus::Crawled as u8 as i16)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;

        if result.rows_affected() == 0 {
            return Err(SavedJobError::NotFound { id });
        }
        Ok(())
    }

    async fn update_analysis(
        &self,
        id: Uuid,
        result: serde_json::Value,
        score: f32,
    ) -> Result<(), SavedJobError> {
        let affected = sqlx::query(
            r#"UPDATE saved_job
               SET analysis_result = $2, match_score = $3,
                   status = $4, analyzed_at = now()
               WHERE id = $1"#,
        )
        .bind(id)
        .bind(&result)
        .bind(score)
        .bind(SavedJobStatus::Analyzed as u8 as i16)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;

        if affected.rows_affected() == 0 {
            return Err(SavedJobError::NotFound { id });
        }
        Ok(())
    }

    async fn list_stale(
        &self,
        older_than: Timestamp,
    ) -> Result<Vec<SavedJob>, SavedJobError> {
        let cutoff = timestamp_to_chrono(older_than);
        let rows = sqlx::query_as::<_, StoreSavedJob>(
            r#"SELECT * FROM saved_job
               WHERE status NOT IN ($1, $2)
                 AND created_at < $3
               ORDER BY created_at ASC"#,
        )
        .bind(SavedJobStatus::Failed as u8 as i16)
        .bind(SavedJobStatus::Expired as u8 as i16)
        .bind(cutoff)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn list_with_s3_keys_by_status(
        &self,
        statuses: &[SavedJobStatus],
    ) -> Result<Vec<SavedJob>, SavedJobError> {
        let status_codes: Vec<i16> = statuses.iter().map(|s| *s as u8 as i16).collect();
        let rows = sqlx::query_as::<_, StoreSavedJob>(
            r#"SELECT * FROM saved_job
               WHERE status = ANY($1)
                 AND markdown_s3_key IS NOT NULL"#,
        )
        .bind(&status_codes)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn clear_s3_key(&self, id: Uuid) -> Result<(), SavedJobError> {
        let result = sqlx::query(
            "UPDATE saved_job SET markdown_s3_key = NULL WHERE id = $1",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;

        if result.rows_affected() == 0 {
            return Err(SavedJobError::NotFound { id });
        }
        Ok(())
    }

    async fn update_title_company(
        &self,
        id: Uuid,
        title: Option<String>,
        company: Option<String>,
    ) -> Result<(), SavedJobError> {
        let result = sqlx::query(
            r#"UPDATE saved_job
               SET title = COALESCE($2, title),
                   company = COALESCE($3, company),
                   updated_at = NOW()
               WHERE id = $1"#,
        )
        .bind(id)
        .bind(title)
        .bind(company)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;

        if result.rows_affected() == 0 {
            return Err(SavedJobError::NotFound { id });
        }
        Ok(())
    }
}
