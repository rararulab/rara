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

//! PostgreSQL implementation of [`JobRepository`].

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::{
    error::SourceError,
    repository::JobRepository,
    types::NormalizedJob,
};

// ---------------------------------------------------------------------------
// DB row types (inlined from rara-model)
// ---------------------------------------------------------------------------

/// PostgreSQL enum mapping for `job_status`.
#[derive(Debug, Clone, sqlx::Type)]
#[sqlx(type_name = "job_status", rename_all = "snake_case")]
pub(crate) enum JobStatusDb {
    Active,
    Archived,
    Closed,
}

/// A job posting row from the `job` table.
#[derive(Debug, Clone, FromRow)]
pub(crate) struct JobRow {
    pub id:              Uuid,
    pub source_job_id:   String,
    pub source_name:     String,
    pub title:           String,
    pub company:         String,
    pub location:        Option<String>,
    pub description:     Option<String>,
    pub url:             Option<String>,
    pub salary_min:      Option<i32>,
    pub salary_max:      Option<i32>,
    pub salary_currency: Option<String>,
    pub tags:            Vec<String>,
    pub status:          JobStatusDb,
    pub raw_data:        Option<serde_json::Value>,
    pub trace_id:        Option<String>,
    pub is_deleted:      bool,
    pub deleted_at:      Option<DateTime<Utc>>,
    pub posted_at:       Option<DateTime<Utc>>,
    pub created_at:      DateTime<Utc>,
    pub updated_at:      DateTime<Utc>,
}

// ===========================================================================
// PgJobRepository (discovery)
// ===========================================================================

/// PostgreSQL-backed job repository.
pub struct PgJobRepository {
    pool: PgPool,
}

impl PgJobRepository {
    pub fn new(pool: PgPool) -> Self { Self { pool } }
}

#[async_trait]
impl JobRepository for PgJobRepository {
    async fn save(&self, job: &NormalizedJob) -> Result<NormalizedJob, SourceError> {
        let db_job: JobRow = JobRow::from(job.clone());

        let row: JobRow = sqlx::query_as(
            r#"
            INSERT INTO job (
                id, source_job_id, source_name, title, company,
                location, description, url,
                salary_min, salary_max, salary_currency,
                tags, status, raw_data, trace_id,
                is_deleted, deleted_at, posted_at,
                created_at, updated_at
            )
            VALUES (
                $1, $2, $3, $4, $5,
                $6, $7, $8,
                $9, $10, $11,
                $12, $13, $14, $15,
                $16, $17, $18,
                $19, $20
            )
            RETURNING *
            "#,
        )
        .bind(db_job.id)
        .bind(&db_job.source_job_id)
        .bind(&db_job.source_name)
        .bind(&db_job.title)
        .bind(&db_job.company)
        .bind(&db_job.location)
        .bind(&db_job.description)
        .bind(&db_job.url)
        .bind(db_job.salary_min)
        .bind(db_job.salary_max)
        .bind(&db_job.salary_currency)
        .bind(&db_job.tags)
        .bind(&db_job.status)
        .bind(&db_job.raw_data)
        .bind(&db_job.trace_id)
        .bind(db_job.is_deleted)
        .bind(db_job.deleted_at)
        .bind(db_job.posted_at)
        .bind(db_job.created_at)
        .bind(db_job.updated_at)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| SourceError::NonRetryable {
            source_name: "pg".to_owned(),
            message:     e.to_string(),
        })?;

        Ok(NormalizedJob::from(row))
    }
}
