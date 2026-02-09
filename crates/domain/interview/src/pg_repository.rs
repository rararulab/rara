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
//! [`crate::repository::InterviewPlanRepository`].

use std::fmt::Write;

use async_trait::async_trait;
use job_domain_shared::id::{ApplicationId, InterviewId};
use sqlx::PgPool;

use crate::{
    convert, db_models,
    error::InterviewError,
    types::{InterviewFilter, InterviewPlan},
};

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
impl crate::repository::InterviewPlanRepository for PgInterviewPlanRepository {
    async fn save(&self, plan: &InterviewPlan) -> Result<InterviewPlan, InterviewError> {
        let store: db_models::InterviewPlan = plan.clone().into();

        let row = sqlx::query_as::<_, db_models::InterviewPlan>(
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
        let row = sqlx::query_as::<_, db_models::InterviewPlan>(
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
        let rows = sqlx::query_as::<_, db_models::InterviewPlan>(
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
            let round_str = convert::interview_round_to_string(round);
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

        let rows = sqlx::query_as::<_, db_models::InterviewPlan>(&sql)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn update(&self, plan: &InterviewPlan) -> Result<InterviewPlan, InterviewError> {
        let store: db_models::InterviewPlan = plan.clone().into();

        let row = sqlx::query_as::<_, db_models::InterviewPlan>(
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use sqlx::postgres::PgPoolOptions;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;

    use super::*;
    use crate::{
        repository::InterviewPlanRepository,
        types::{InterviewRound, InterviewTaskStatus, PrepMaterials},
    };

    async fn connect_pool(url: &str) -> sqlx::PgPool {
        let mut last_err: Option<sqlx::Error> = None;
        for _ in 0..30 {
            match PgPoolOptions::new()
                .max_connections(5)
                .acquire_timeout(Duration::from_secs(10))
                .connect(url)
                .await
            {
                Ok(pool) => return pool,
                Err(e) => {
                    last_err = Some(e);
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
        }
        panic!("failed to connect to postgres: {last_err:?}");
    }

    async fn setup_pool() -> (sqlx::PgPool, testcontainers::ContainerAsync<Postgres>) {
        let container = Postgres::default().start().await.unwrap();
        let host = container.get_host().await.unwrap();
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
        let pool = connect_pool(&url).await;

        // Ensure gen_random_uuid() is available (older PG images need pgcrypto).
        sqlx::raw_sql("CREATE EXTENSION IF NOT EXISTS \"pgcrypto\"")
            .execute(&pool)
            .await
            .unwrap();

        // Run migrations in order
        let migrations: &[&str] = &[
            include_str!("../../../job-model/migrations/20260127000000_init.sql"),
            include_str!("../../../job-model/migrations/20260208000000_domain_models.sql"),
            include_str!("../../../job-model/migrations/20260209000000_resume_version_mgmt.sql"),
            include_str!("../../../job-model/migrations/20260210000000_schema_alignment.sql"),
            include_str!("../../../job-model/migrations/20260211000000_notify_priority.sql"),
        ];

        for sql in migrations {
            sqlx::raw_sql(sql).execute(&pool).await.unwrap();
        }

        // The scheduler migration references set_updated_at() but the
        // function was created as trigger_set_updated_at() in the domain
        // migration. Fix the reference before executing.
        let scheduler_sql =
            include_str!("../../../job-model/migrations/20260211000001_scheduler_tables.sql")
                .replace(
                    "FUNCTION set_updated_at()",
                    "FUNCTION trigger_set_updated_at()",
                );
        sqlx::raw_sql(&scheduler_sql).execute(&pool).await.unwrap();

        // Convert domain enum columns to SMALLINT codes.
        let domain_int_migrations: &[&str] = &[
            include_str!("../../../job-model/migrations/20260212000000_application_int_enums.sql"),
            include_str!("../../../job-model/migrations/20260212000001_interview_int_enums.sql"),
            include_str!("../../../job-model/migrations/20260212000002_notify_int_enums.sql"),
            include_str!("../../../job-model/migrations/20260212000003_resume_int_enums.sql"),
            include_str!("../../../job-model/migrations/20260212000004_scheduler_int_enums.sql"),
        ];
        for sql in domain_int_migrations {
            sqlx::raw_sql(sql).execute(&pool).await.unwrap();
        }

        // Insert job and application rows to satisfy FK constraints
        sqlx::query(
            r#"INSERT INTO job (id, source_job_id, source_name, title, company)
               VALUES ('00000000-0000-0000-0000-000000000001', 'test-1', 'manual', 'SWE', 'TestCo')"#,
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            r#"INSERT INTO application (id, job_id, channel, status)
               VALUES ('00000000-0000-0000-0000-000000000002',
                       '00000000-0000-0000-0000-000000000001',
                       0,
                       0)"#,
        )
        .execute(&pool)
        .await
        .unwrap();

        (pool, container)
    }

    fn app_id() -> ApplicationId {
        ApplicationId::from(
            "00000000-0000-0000-0000-000000000002"
                .parse::<uuid::Uuid>()
                .unwrap(),
        )
    }

    fn make_plan(application_id: ApplicationId) -> InterviewPlan {
        let now = jiff::Timestamp::now();
        InterviewPlan {
            id: InterviewId::new(),
            application_id,
            title: "Technical Interview".into(),
            company: "Acme Corp".into(),
            position: "Senior SWE".into(),
            job_description: Some("Build distributed systems".into()),
            round: InterviewRound::Technical,
            scheduled_at: None,
            task_status: InterviewTaskStatus::Pending,
            prep_materials: PrepMaterials::default(),
            notes: Some("Prepare for systems design".into()),
            trace_id: None,
            is_deleted: false,
            deleted_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn save_and_find_by_id() {
        let (pool, _container) = setup_pool().await;
        let repo = PgInterviewPlanRepository::new(pool);

        let plan = make_plan(app_id());
        let saved = repo.save(&plan).await.unwrap();
        assert_eq!(saved.title, "Technical Interview");
        assert_eq!(saved.company, "Acme Corp");

        let found = repo.find_by_id(plan.id).await.unwrap().unwrap();
        assert_eq!(found.id, plan.id);
        assert_eq!(found.round, InterviewRound::Technical);
    }

    #[tokio::test]
    async fn update_interview_plan() {
        let (pool, _container) = setup_pool().await;
        let repo = PgInterviewPlanRepository::new(pool);

        let plan = make_plan(app_id());
        repo.save(&plan).await.unwrap();

        let mut updated_plan = plan.clone();
        updated_plan.title = "Updated Interview".into();
        updated_plan.task_status = InterviewTaskStatus::InProgress;
        updated_plan.round = InterviewRound::SystemDesign;

        let updated = repo.update(&updated_plan).await.unwrap();
        assert_eq!(updated.title, "Updated Interview");
        assert_eq!(updated.task_status, InterviewTaskStatus::InProgress);
        assert_eq!(updated.round, InterviewRound::SystemDesign);
    }

    #[tokio::test]
    async fn soft_delete() {
        let (pool, _container) = setup_pool().await;
        let repo = PgInterviewPlanRepository::new(pool);

        let plan = make_plan(app_id());
        repo.save(&plan).await.unwrap();

        repo.delete(plan.id).await.unwrap();
        let found = repo.find_by_id(plan.id).await.unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn soft_delete_not_found() {
        let (pool, _container) = setup_pool().await;
        let repo = PgInterviewPlanRepository::new(pool);

        let result = repo.delete(InterviewId::new()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn find_by_application() {
        let (pool, _container) = setup_pool().await;
        let repo = PgInterviewPlanRepository::new(pool);

        let plan1 = make_plan(app_id());
        repo.save(&plan1).await.unwrap();

        let mut plan2 = make_plan(app_id());
        plan2.title = "Behavioral Interview".into();
        plan2.round = InterviewRound::Behavioral;
        repo.save(&plan2).await.unwrap();

        let results = repo.find_by_application(app_id()).await.unwrap();
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn find_all_with_status_filter() {
        let (pool, _container) = setup_pool().await;
        let repo = PgInterviewPlanRepository::new(pool);

        let plan1 = make_plan(app_id());
        repo.save(&plan1).await.unwrap();

        let mut plan2 = make_plan(app_id());
        plan2.task_status = InterviewTaskStatus::Completed;
        repo.save(&plan2).await.unwrap();

        let filter = InterviewFilter {
            task_status: Some(InterviewTaskStatus::Pending),
            ..Default::default()
        };
        let results = repo.find_all(&filter).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].task_status, InterviewTaskStatus::Pending);
    }

    #[tokio::test]
    async fn find_all_with_company_filter() {
        let (pool, _container) = setup_pool().await;
        let repo = PgInterviewPlanRepository::new(pool);

        let plan1 = make_plan(app_id());
        repo.save(&plan1).await.unwrap();

        let mut plan2 = make_plan(app_id());
        plan2.company = "Other Corp".into();
        repo.save(&plan2).await.unwrap();

        let filter = InterviewFilter {
            company: Some("Acme Corp".into()),
            ..Default::default()
        };
        let results = repo.find_all(&filter).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].company, "Acme Corp");
    }

    #[tokio::test]
    async fn find_all_with_round_filter() {
        let (pool, _container) = setup_pool().await;
        let repo = PgInterviewPlanRepository::new(pool);

        let plan1 = make_plan(app_id());
        repo.save(&plan1).await.unwrap();

        let mut plan2 = make_plan(app_id());
        plan2.round = InterviewRound::Behavioral;
        repo.save(&plan2).await.unwrap();

        let filter = InterviewFilter {
            round: Some(InterviewRound::Technical),
            ..Default::default()
        };
        let results = repo.find_all(&filter).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].round, InterviewRound::Technical);
    }
}
