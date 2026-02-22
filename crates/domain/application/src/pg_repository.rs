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
//! [`crate::repository::ApplicationRepository`].

use std::fmt::Write;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rara_domain_shared::id::ApplicationId;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::{
    error::ApplicationError,
    types::{Application, ApplicationFilter, StatusChangeRecord},
};

// ---------------------------------------------------------------------------
// DB row types (inlined from rara-model)
// ---------------------------------------------------------------------------

/// A job application record (DB row).
#[derive(Debug, Clone, FromRow)]
pub(crate) struct ApplicationRow {
    pub id:           Uuid,
    pub job_id:       Uuid,
    pub resume_id:    Option<Uuid>,
    pub channel:      i16,
    pub status:       i16,
    pub cover_letter: Option<String>,
    pub notes:        Option<String>,
    pub tags:         Vec<String>,
    pub priority:     i16,
    pub trace_id:     Option<String>,
    pub is_deleted:   bool,
    pub deleted_at:   Option<DateTime<Utc>>,
    pub submitted_at: Option<DateTime<Utc>>,
    pub created_at:   DateTime<Utc>,
    pub updated_at:   DateTime<Utc>,
}

/// An immutable record of an application status transition (DB row).
#[derive(Debug, Clone, FromRow)]
pub(crate) struct ApplicationStatusHistoryRow {
    pub id:             Uuid,
    pub application_id: Uuid,
    pub from_status:    Option<i16>,
    pub to_status:      i16,
    pub changed_by:     Option<String>,
    pub note:           Option<String>,
    pub trace_id:       Option<String>,
    pub created_at:     DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// PgApplicationRepository
// ---------------------------------------------------------------------------

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
impl crate::repository::ApplicationRepository for PgApplicationRepository {
    async fn save(&self, app: &Application) -> Result<Application, ApplicationError> {
        let store: ApplicationRow = app.clone().into();

        let row = sqlx::query_as::<_, ApplicationRow>(
            r#"INSERT INTO application
                   (id, job_id, resume_id, channel, status, cover_letter, notes,
                    tags, priority, trace_id, is_deleted, submitted_at, created_at, updated_at)
               VALUES
                   ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
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

    async fn find_by_id(&self, id: ApplicationId) -> Result<Option<Application>, ApplicationError> {
        let row = sqlx::query_as::<_, ApplicationRow>(
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
            let status_code = *status as u8 as i16;
            let _ = write!(sql, " AND status = {status_code}");
        }

        if let Some(ref job_id) = filter.job_id {
            let _ = write!(sql, " AND job_id = '{}'", job_id.into_inner());
        }

        if let Some(ref resume_id) = filter.resume_id {
            let _ = write!(sql, " AND resume_id = '{}'", resume_id.into_inner());
        }

        if let Some(ref channel) = filter.channel {
            let channel_code = *channel as u8 as i16;
            let _ = write!(sql, " AND channel = {channel_code}");
        }

        if let Some(ref priority) = filter.priority {
            let priority_code = *priority as u8 as i16;
            let _ = write!(sql, " AND priority = {priority_code}");
        }

        if let Some(ref created_after) = filter.created_after {
            let _ = write!(sql, " AND created_at >= '{created_after}'");
        }

        if let Some(ref created_before) = filter.created_before {
            let _ = write!(sql, " AND created_at <= '{created_before}'");
        }

        sql.push_str(" ORDER BY created_at DESC");

        let rows = sqlx::query_as::<_, ApplicationRow>(&sql)
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
        let store: ApplicationRow = app.clone().into();

        let row = sqlx::query_as::<_, ApplicationRow>(
            r#"UPDATE application
               SET job_id = $2, resume_id = $3, channel = $4, status = $5,
                   cover_letter = $6, notes = $7, tags = $8, priority = $9,
                   trace_id = $10, submitted_at = $11
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
            "UPDATE application SET is_deleted = TRUE, deleted_at = now() WHERE id = $1 AND \
             is_deleted = FALSE",
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
        let store: ApplicationStatusHistoryRow = record.clone().into();

        sqlx::query(
            r#"INSERT INTO application_status_history
                   (id, application_id, from_status, to_status, changed_by, note, trace_id, created_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#,
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
        let rows = sqlx::query_as::<_, ApplicationStatusHistoryRow>(
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
