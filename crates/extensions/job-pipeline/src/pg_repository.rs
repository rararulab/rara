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

//! PostgreSQL-backed implementation of [`crate::repository::PipelineRepository`].

use async_trait::async_trait;
use rara_domain_shared::convert::timestamp_opt_to_chrono;
use snafu::ResultExt as _;
use sqlx::PgPool;
use uuid::Uuid;

use crate::repository::{DatabaseSnafu, PipelineRepoError, PipelineRepository};
use crate::types::{
    DiscoveredJob, DiscoveredJobAction, DiscoveredJobRow, DiscoveredJobsActionCounts,
    DiscoveredJobsStats, PipelineEvent, PipelineEventRow, PipelineRun, PipelineRunRow,
};

// ---------------------------------------------------------------------------
// PgPipelineRepository
// ---------------------------------------------------------------------------

/// PostgreSQL implementation of the pipeline repository.
pub struct PgPipelineRepository {
    pool: PgPool,
}

impl PgPipelineRepository {
    /// Create a new repository backed by the given connection pool.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl PipelineRepository for PgPipelineRepository {
    async fn create_run(&self) -> Result<PipelineRun, PipelineRepoError> {
        let row = sqlx::query_as::<_, PipelineRunRow>(
            r#"INSERT INTO pipeline_runs DEFAULT VALUES
               RETURNING *"#,
        )
        .fetch_one(&self.pool)
        .await
        .context(DatabaseSnafu)?;

        Ok(row.into())
    }

    async fn update_run(&self, run: &PipelineRun) -> Result<(), PipelineRepoError> {
        let finished_at = timestamp_opt_to_chrono(run.finished_at);

        sqlx::query(
            r#"UPDATE pipeline_runs
               SET status = $2,
                   finished_at = $3,
                   jobs_found = $4,
                   jobs_scored = $5,
                   jobs_applied = $6,
                   jobs_notified = $7,
                   summary = $8,
                   error = $9
               WHERE id = $1"#,
        )
        .bind(run.id)
        .bind(run.status as u8 as i16)
        .bind(finished_at)
        .bind(run.jobs_found)
        .bind(run.jobs_scored)
        .bind(run.jobs_applied)
        .bind(run.jobs_notified)
        .bind(&run.summary)
        .bind(&run.error)
        .execute(&self.pool)
        .await
        .context(DatabaseSnafu)?;

        Ok(())
    }

    async fn get_run(&self, id: Uuid) -> Result<Option<PipelineRun>, PipelineRepoError> {
        let row = sqlx::query_as::<_, PipelineRunRow>(
            "SELECT * FROM pipeline_runs WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .context(DatabaseSnafu)?;

        Ok(row.map(Into::into))
    }

    async fn list_runs(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<PipelineRun>, PipelineRepoError> {
        let rows = sqlx::query_as::<_, PipelineRunRow>(
            r#"SELECT * FROM pipeline_runs
               ORDER BY started_at DESC
               LIMIT $1 OFFSET $2"#,
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .context(DatabaseSnafu)?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn insert_event(
        &self,
        run_id: Uuid,
        seq: i32,
        event_type: &str,
        payload: serde_json::Value,
    ) -> Result<(), PipelineRepoError> {
        sqlx::query(
            r#"INSERT INTO pipeline_events (run_id, seq, event_type, payload)
               VALUES ($1, $2, $3, $4)"#,
        )
        .bind(run_id)
        .bind(seq)
        .bind(event_type)
        .bind(payload)
        .execute(&self.pool)
        .await
        .context(DatabaseSnafu)?;

        Ok(())
    }

    async fn get_events(&self, run_id: Uuid) -> Result<Vec<PipelineEvent>, PipelineRepoError> {
        let rows = sqlx::query_as::<_, PipelineEventRow>(
            r#"SELECT * FROM pipeline_events
               WHERE run_id = $1
               ORDER BY seq ASC"#,
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await
        .context(DatabaseSnafu)?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn insert_discovered_job(
        &self,
        run_id: Uuid,
        title: &str,
        company: Option<&str>,
        location: Option<&str>,
        url: Option<&str>,
        description: Option<&str>,
        score: Option<i32>,
        action: DiscoveredJobAction,
        date_posted: Option<&str>,
    ) -> Result<DiscoveredJob, PipelineRepoError> {
        let row = sqlx::query_as::<_, DiscoveredJobRow>(
            r#"INSERT INTO pipeline_discovered_jobs
                   (run_id, title, company, location, url, description, score, action, date_posted)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
               RETURNING *"#,
        )
        .bind(run_id)
        .bind(title)
        .bind(company)
        .bind(location)
        .bind(url)
        .bind(description)
        .bind(score)
        .bind(action as u8 as i16)
        .bind(date_posted)
        .fetch_one(&self.pool)
        .await
        .context(DatabaseSnafu)?;

        Ok(row.into())
    }

    async fn list_discovered_jobs(
        &self,
        run_id: Uuid,
    ) -> Result<Vec<DiscoveredJob>, PipelineRepoError> {
        let rows = sqlx::query_as::<_, DiscoveredJobRow>(
            r#"SELECT * FROM pipeline_discovered_jobs
               WHERE run_id = $1
               ORDER BY score DESC NULLS LAST"#,
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await
        .context(DatabaseSnafu)?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn list_unscored_discovered_jobs(
        &self,
        run_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<DiscoveredJob>, PipelineRepoError> {
        let rows = sqlx::query_as::<_, DiscoveredJobRow>(
            r#"SELECT * FROM pipeline_discovered_jobs
               WHERE run_id = $1
                 AND score IS NULL
               ORDER BY created_at ASC
               LIMIT $2 OFFSET $3"#,
        )
        .bind(run_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .context(DatabaseSnafu)?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn update_discovered_job_score_action(
        &self,
        id: Uuid,
        score: Option<i32>,
        action: Option<DiscoveredJobAction>,
    ) -> Result<Option<DiscoveredJob>, PipelineRepoError> {
        let action_i16 = action.map(|a| a as u8 as i16);
        let row = sqlx::query_as::<_, DiscoveredJobRow>(
            r#"UPDATE pipeline_discovered_jobs
               SET score = COALESCE($2, score),
                   action = COALESCE($3, action)
               WHERE id = $1
               RETURNING *"#,
        )
        .bind(id)
        .bind(score)
        .bind(action_i16)
        .fetch_optional(&self.pool)
        .await
        .context(DatabaseSnafu)?;

        Ok(row.map(Into::into))
    }

    async fn list_all_discovered_jobs(
        &self,
        action: Option<DiscoveredJobAction>,
        min_score: Option<i32>,
        max_score: Option<i32>,
        run_id: Option<Uuid>,
        sort_by: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<DiscoveredJob>, PipelineRepoError> {
        let mut sql = String::from("SELECT * FROM pipeline_discovered_jobs WHERE 1=1");
        let mut param_idx: usize = 0;

        if action.is_some() {
            param_idx += 1;
            sql.push_str(&format!(" AND action = ${param_idx}"));
        }
        if min_score.is_some() {
            param_idx += 1;
            sql.push_str(&format!(" AND score >= ${param_idx}"));
        }
        if max_score.is_some() {
            param_idx += 1;
            sql.push_str(&format!(" AND score <= ${param_idx}"));
        }
        if run_id.is_some() {
            param_idx += 1;
            sql.push_str(&format!(" AND run_id = ${param_idx}"));
        }

        let order = match sort_by {
            Some("score") => "ORDER BY score DESC NULLS LAST",
            _ => "ORDER BY created_at DESC",
        };
        sql.push(' ');
        sql.push_str(order);

        param_idx += 1;
        let limit_idx = param_idx;
        param_idx += 1;
        let offset_idx = param_idx;
        sql.push_str(&format!(" LIMIT ${limit_idx} OFFSET ${offset_idx}"));

        let mut query = sqlx::query_as::<_, DiscoveredJobRow>(&sql);
        if let Some(a) = action {
            query = query.bind(a as u8 as i16);
        }
        if let Some(v) = min_score {
            query = query.bind(v);
        }
        if let Some(v) = max_score {
            query = query.bind(v);
        }
        if let Some(v) = run_id {
            query = query.bind(v);
        }
        query = query.bind(limit).bind(offset);

        let rows = query.fetch_all(&self.pool).await.context(DatabaseSnafu)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn count_discovered_jobs(
        &self,
        action: Option<DiscoveredJobAction>,
        min_score: Option<i32>,
        max_score: Option<i32>,
        run_id: Option<Uuid>,
    ) -> Result<i64, PipelineRepoError> {
        let mut sql =
            String::from("SELECT COUNT(*) as count FROM pipeline_discovered_jobs WHERE 1=1");
        let mut param_idx: usize = 0;

        if action.is_some() {
            param_idx += 1;
            sql.push_str(&format!(" AND action = ${param_idx}"));
        }
        if min_score.is_some() {
            param_idx += 1;
            sql.push_str(&format!(" AND score >= ${param_idx}"));
        }
        if max_score.is_some() {
            param_idx += 1;
            sql.push_str(&format!(" AND score <= ${param_idx}"));
        }
        if run_id.is_some() {
            param_idx += 1;
            sql.push_str(&format!(" AND run_id = ${param_idx}"));
        }

        let mut query = sqlx::query_scalar::<_, i64>(&sql);
        if let Some(a) = action {
            query = query.bind(a as u8 as i16);
        }
        if let Some(v) = min_score {
            query = query.bind(v);
        }
        if let Some(v) = max_score {
            query = query.bind(v);
        }
        if let Some(v) = run_id {
            query = query.bind(v);
        }

        let count = query.fetch_one(&self.pool).await.context(DatabaseSnafu)?;
        Ok(count)
    }

    async fn discovered_jobs_stats(&self) -> Result<DiscoveredJobsStats, PipelineRepoError> {
        let row = sqlx::query_as::<_, StatsRow>(
            r#"SELECT
                   COUNT(*) AS total,
                   COUNT(*) FILTER (WHERE action = 0) AS discovered,
                   COUNT(*) FILTER (WHERE action = 1) AS notified,
                   COUNT(*) FILTER (WHERE action = 2) AS applied,
                   COUNT(*) FILTER (WHERE action = 3) AS skipped,
                   COUNT(*) FILTER (WHERE score IS NOT NULL) AS scored_count,
                   AVG(score) FILTER (WHERE score IS NOT NULL) AS avg_score
               FROM pipeline_discovered_jobs"#,
        )
        .fetch_one(&self.pool)
        .await
        .context(DatabaseSnafu)?;

        Ok(DiscoveredJobsStats {
            total: row.total,
            by_action: DiscoveredJobsActionCounts {
                discovered: row.discovered,
                notified: row.notified,
                applied: row.applied,
                skipped: row.skipped,
            },
            scored_count: row.scored_count,
            avg_score: row.avg_score,
        })
    }
}

#[derive(Debug, sqlx::FromRow)]
struct StatsRow {
    total: i64,
    discovered: i64,
    notified: i64,
    applied: i64,
    skipped: i64,
    scored_count: i64,
    avg_score: Option<f64>,
}
