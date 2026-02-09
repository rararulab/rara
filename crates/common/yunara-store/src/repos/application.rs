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
//! [`job_domain_application::repository::ApplicationRepository`].

use std::fmt::Write;

use async_trait::async_trait;
use job_domain_application::{
    error::ApplicationError,
    types::{Application, ApplicationFilter, StatusChangeRecord},
};
use job_domain_core::id::ApplicationId;
use sqlx::PgPool;

use crate::models;

/// PostgreSQL implementation of the application repository.
pub struct PgApplicationRepository {
    pool: PgPool,
}

impl PgApplicationRepository {
    /// Create a new repository backed by the given connection pool.
    #[must_use]
    pub fn new(pool: PgPool) -> Self { Self { pool } }
}

/// Map a `sqlx::Error` into an `ApplicationError::RepositoryError`.
fn map_err(e: sqlx::Error) -> ApplicationError {
    ApplicationError::RepositoryError {
        message: e.to_string(),
    }
}

#[async_trait]
impl job_domain_application::repository::ApplicationRepository for PgApplicationRepository {
    async fn save(&self, app: &Application) -> Result<Application, ApplicationError> {
        let store: models::application::Application = app.clone().into();

        let row = sqlx::query_as::<_, models::application::Application>(
            r#"INSERT INTO application
                   (id, job_id, resume_id, channel, status, cover_letter, notes,
                    tags, priority, trace_id, is_deleted, submitted_at, created_at, updated_at)
               VALUES
                   ($1, $2, $3, $4::application_channel, $5::application_status,
                    $6, $7, $8, $9::application_priority, $10, $11, $12, $13, $14)
               RETURNING *"#,
        )
        .bind(store.id)
        .bind(store.job_id)
        .bind(store.resume_id)
        .bind(&store.channel)
        .bind(&store.status)
        .bind(&store.cover_letter)
        .bind(&store.notes)
        .bind(&store.tags)
        .bind(&store.priority)
        .bind(&store.trace_id)
        .bind(store.is_deleted)
        .bind(store.submitted_at)
        .bind(store.created_at)
        .bind(store.updated_at)
        .fetch_one(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(row.into())
    }

    async fn find_by_id(
        &self,
        id: ApplicationId,
    ) -> Result<Option<Application>, ApplicationError> {
        let row = sqlx::query_as::<_, models::application::Application>(
            "SELECT * FROM application WHERE id = $1 AND is_deleted = FALSE",
        )
        .bind(id.into_inner())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(row.map(Into::into))
    }

    async fn find_all(
        &self,
        filter: &ApplicationFilter,
    ) -> Result<Vec<Application>, ApplicationError> {
        let mut sql = String::from("SELECT * FROM application WHERE is_deleted = FALSE");

        if let Some(ref status) = filter.status {
            let store_status: models::application::ApplicationStatus = (*status).into();
            let _ = write!(sql, " AND status = '{store_status}'::application_status");
        }

        if let Some(ref job_id) = filter.job_id {
            let _ = write!(sql, " AND job_id = '{}'", job_id.into_inner());
        }

        if let Some(ref resume_id) = filter.resume_id {
            let _ = write!(sql, " AND resume_id = '{}'", resume_id.into_inner());
        }

        if let Some(ref channel) = filter.channel {
            let store_channel: models::application::ApplicationChannel = (*channel).into();
            let _ = write!(
                sql,
                " AND channel = '{store_channel}'::application_channel"
            );
        }

        if let Some(ref priority) = filter.priority {
            let store_priority: models::application::ApplicationPriority = (*priority).into();
            let _ = write!(
                sql,
                " AND priority = '{store_priority}'::application_priority"
            );
        }

        if let Some(ref created_after) = filter.created_after {
            let _ = write!(sql, " AND created_at >= '{created_after}'");
        }

        if let Some(ref created_before) = filter.created_before {
            let _ = write!(sql, " AND created_at <= '{created_before}'");
        }

        sql.push_str(" ORDER BY created_at DESC");

        let rows = sqlx::query_as::<_, models::application::Application>(&sql)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?;

        let mut results: Vec<Application> = rows.into_iter().map(Into::into).collect();

        // Apply tag filter in-memory.
        if let Some(ref required_tags) = filter.tags {
            results.retain(|a| required_tags.iter().all(|t| a.tags.contains(t)));
        }

        Ok(results)
    }

    async fn update(&self, app: &Application) -> Result<Application, ApplicationError> {
        let store: models::application::Application = app.clone().into();

        let row = sqlx::query_as::<_, models::application::Application>(
            r#"UPDATE application
               SET job_id = $2, resume_id = $3, channel = $4::application_channel,
                   status = $5::application_status, cover_letter = $6, notes = $7,
                   tags = $8, priority = $9::application_priority, trace_id = $10,
                   submitted_at = $11
               WHERE id = $1 AND is_deleted = FALSE
               RETURNING *"#,
        )
        .bind(store.id)
        .bind(store.job_id)
        .bind(store.resume_id)
        .bind(&store.channel)
        .bind(&store.status)
        .bind(&store.cover_letter)
        .bind(&store.notes)
        .bind(&store.tags)
        .bind(&store.priority)
        .bind(&store.trace_id)
        .bind(store.submitted_at)
        .fetch_one(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(row.into())
    }

    async fn delete(&self, id: ApplicationId) -> Result<(), ApplicationError> {
        let result = sqlx::query(
            "UPDATE application SET is_deleted = TRUE, deleted_at = now() WHERE id = $1 AND is_deleted = FALSE",
        )
        .bind(id.into_inner())
        .execute(&self.pool)
        .await
        .map_err(map_err)?;

        if result.rows_affected() == 0 {
            return Err(ApplicationError::NotFound { id });
        }
        Ok(())
    }

    async fn save_status_change(
        &self,
        record: &StatusChangeRecord,
    ) -> Result<(), ApplicationError> {
        let store: models::application::ApplicationStatusHistory = record.clone().into();

        sqlx::query(
            r#"INSERT INTO application_status_history
                   (id, application_id, from_status, to_status, changed_by, note, trace_id, created_at)
               VALUES ($1, $2, $3::application_status, $4::application_status, $5, $6, $7, $8)"#,
        )
        .bind(store.id)
        .bind(store.application_id)
        .bind(&store.from_status)
        .bind(&store.to_status)
        .bind(&store.changed_by)
        .bind(&store.note)
        .bind(&store.trace_id)
        .bind(store.created_at)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(())
    }

    async fn get_status_history(
        &self,
        application_id: ApplicationId,
    ) -> Result<Vec<StatusChangeRecord>, ApplicationError> {
        let rows = sqlx::query_as::<_, models::application::ApplicationStatusHistory>(
            r#"SELECT * FROM application_status_history
               WHERE application_id = $1
               ORDER BY created_at ASC"#,
        )
        .bind(application_id.into_inner())
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(rows.into_iter().map(Into::into).collect())
    }
}
