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
//! [`super::repository::InterviewPlanRepository`].

use std::fmt::Write;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rara_domain_shared::id::{ApplicationId, InterviewId};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use super::{
    error::InterviewError,
    types::{self, InterviewFilter, InterviewPlan},
};

// ---------------------------------------------------------------------------
// DB row types (inlined from rara-model)
// ---------------------------------------------------------------------------

/// An interview preparation plan (DB row).
#[derive(Debug, Clone, FromRow)]
pub(super) struct InterviewPlanRow {
    pub id:              Uuid,
    pub application_id:  Uuid,
    pub title:           String,
    pub company:         String,
    pub position:        String,
    pub job_description: Option<String>,
    pub round:           String,
    pub description:     Option<String>,
    pub scheduled_at:    Option<DateTime<Utc>>,
    pub task_status:     i16,
    pub materials:       Option<serde_json::Value>,
    pub notes:           Option<String>,
    pub trace_id:        Option<String>,
    pub is_deleted:      bool,
    pub deleted_at:      Option<DateTime<Utc>>,
    pub created_at:      DateTime<Utc>,
    pub updated_at:      DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// PgInterviewPlanRepository
// ---------------------------------------------------------------------------

/// PostgreSQL implementation of the interview plan repository.
pub struct PgInterviewPlanRepository {
    pool: PgPool,
}

impl PgInterviewPlanRepository {
    /// Create a new repository backed by the given connection pool.
    #[must_use]
    pub fn new(pool: PgPool) -> Self { Self { pool } }
}

/// Map a `sqlx::Error` into an `InterviewError::RepositoryError`.
fn map_err(e: sqlx::Error) -> InterviewError {
    InterviewError::RepositoryError {
        message: e.to_string(),
    }
}

#[async_trait]
impl super::repository::InterviewPlanRepository for PgInterviewPlanRepository {
    async fn save(&self, plan: &InterviewPlan) -> Result<InterviewPlan, InterviewError> {
        let store: InterviewPlanRow = plan.clone().into();

        let row = sqlx::query_as::<_, InterviewPlanRow>(
            r#"INSERT INTO interview_plan
                   (id, application_id, title, company, position, job_description, round,
                    description, scheduled_at, task_status, materials, notes, trace_id,
                    is_deleted, created_at, updated_at)
               VALUES
                   ($1, $2, $3, $4, $5, $6, $7,
                    $8, $9, $10, $11, $12, $13,
                    $14, $15, $16)
               RETURNING *"#,
        )
        .bind(store.id)
        .bind(store.application_id)
        .bind(&store.title)
        .bind(&store.company)
        .bind(&store.position)
        .bind(&store.job_description)
        .bind(&store.round)
        .bind(&store.description)
        .bind(store.scheduled_at)
        .bind(&store.task_status)
        .bind(&store.materials)
        .bind(&store.notes)
        .bind(&store.trace_id)
        .bind(store.is_deleted)
        .bind(store.created_at)
        .bind(store.updated_at)
        .fetch_one(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(row.into())
    }

    async fn find_by_id(&self, id: InterviewId) -> Result<Option<InterviewPlan>, InterviewError> {
        let row = sqlx::query_as::<_, InterviewPlanRow>(
            "SELECT * FROM interview_plan WHERE id = $1 AND is_deleted = FALSE",
        )
        .bind(id.into_inner())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(row.map(Into::into))
    }

    async fn find_by_application(
        &self,
        app_id: ApplicationId,
    ) -> Result<Vec<InterviewPlan>, InterviewError> {
        let rows = sqlx::query_as::<_, InterviewPlanRow>(
            r#"SELECT * FROM interview_plan
               WHERE application_id = $1 AND is_deleted = FALSE
               ORDER BY created_at ASC"#,
        )
        .bind(app_id.into_inner())
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn find_all(
        &self,
        filter: &InterviewFilter,
    ) -> Result<Vec<InterviewPlan>, InterviewError> {
        let mut sql = String::from("SELECT * FROM interview_plan WHERE is_deleted = FALSE");

        if let Some(ref app_id) = filter.application_id {
            let _ = write!(sql, " AND application_id = '{}'", app_id.into_inner());
        }

        if let Some(ref company) = filter.company {
            // Simple escaping for single quotes in company names.
            let escaped = company.replace('\'', "''");
            let _ = write!(sql, " AND company = '{escaped}'");
        }

        if let Some(ref task_status) = filter.task_status {
            let status_code = *task_status as u8 as i16;
            let _ = write!(sql, " AND task_status = {status_code}");
        }

        if let Some(ref round) = filter.round {
            let round_str = types::interview_round_to_string(round);
            let escaped = round_str.replace('\'', "''");
            let _ = write!(sql, " AND round = '{escaped}'");
        }

        if let Some(ref scheduled_after) = filter.scheduled_after {
            let _ = write!(sql, " AND scheduled_at >= '{scheduled_after}'");
        }

        if let Some(ref scheduled_before) = filter.scheduled_before {
            let _ = write!(sql, " AND scheduled_at <= '{scheduled_before}'");
        }

        sql.push_str(" ORDER BY created_at DESC");

        let rows = sqlx::query_as::<_, InterviewPlanRow>(&sql)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn update(&self, plan: &InterviewPlan) -> Result<InterviewPlan, InterviewError> {
        let store: InterviewPlanRow = plan.clone().into();

        let row = sqlx::query_as::<_, InterviewPlanRow>(
            r#"UPDATE interview_plan
               SET title = $2, company = $3, position = $4, job_description = $5,
                   round = $6, description = $7, scheduled_at = $8,
                   task_status = $9, materials = $10,
                   notes = $11, trace_id = $12
               WHERE id = $1 AND is_deleted = FALSE
               RETURNING *"#,
        )
        .bind(store.id)
        .bind(&store.title)
        .bind(&store.company)
        .bind(&store.position)
        .bind(&store.job_description)
        .bind(&store.round)
        .bind(&store.description)
        .bind(store.scheduled_at)
        .bind(&store.task_status)
        .bind(&store.materials)
        .bind(&store.notes)
        .bind(&store.trace_id)
        .fetch_one(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(row.into())
    }

    async fn delete(&self, id: InterviewId) -> Result<(), InterviewError> {
        let result = sqlx::query(
            "UPDATE interview_plan SET is_deleted = TRUE, deleted_at = now() WHERE id = $1 AND \
             is_deleted = FALSE",
        )
        .bind(id.into_inner())
        .execute(&self.pool)
        .await
        .map_err(map_err)?;

        if result.rows_affected() == 0 {
            return Err(InterviewError::NotFound {
                id: id.into_inner(),
            });
        }
        Ok(())
    }
}
