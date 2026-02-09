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

//! Application-level service for interview plan management.
//!
//! [`InterviewService`] orchestrates domain logic -- plan creation,
//! AI prep generation, status transitions, and filtering -- on top of
//! an [`InterviewPlanRepository`] implementation.

use std::sync::Arc;

use jiff::Timestamp;
use job_domain_core::id::{ApplicationId, InterviewId};
use tracing::instrument;

use crate::{
    error::{
        InterviewError, InvalidStatusTransitionSnafu, NotFoundSnafu, PrepGenerationFailedSnafu,
        ValidationSnafu,
    },
    prep_generator::PrepGenerator,
    repository::InterviewPlanRepository,
    types::{
        CreateInterviewPlanRequest, InterviewFilter, InterviewPlan, InterviewTaskStatus,
        PrepGenerationRequest, PrepMaterials, UpdateInterviewPlanRequest,
    },
};

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

/// High-level service for interview plan CRUD and AI prep generation.
pub struct InterviewService {
    repo:           Arc<dyn InterviewPlanRepository>,
    prep_generator: Option<Arc<dyn PrepGenerator>>,
}

impl InterviewService {
    /// Create a new service backed by the given repository.
    ///
    /// If `prep_generator` is `Some`, the service can auto-generate
    /// preparation materials when creating or refreshing plans.
    #[must_use]
    pub fn new(
        repo: Arc<dyn InterviewPlanRepository>,
        prep_generator: Option<Arc<dyn PrepGenerator>>,
    ) -> Self {
        Self {
            repo,
            prep_generator,
        }
    }

    // -- Create -------------------------------------------------------------

    /// Create a new interview plan.
    ///
    /// If a [`PrepGenerator`] is configured and the request contains
    /// enough context, prep materials are generated automatically.
    #[instrument(skip(self, req))]
    pub async fn create_plan(
        &self,
        req: CreateInterviewPlanRequest,
    ) -> Result<InterviewPlan, InterviewError> {
        Self::validate_create_request(&req)?;

        let now = Timestamp::now();
        let plan = InterviewPlan {
            id:              InterviewId::new(),
            application_id:  req.application_id,
            title:           req.title,
            company:         req.company,
            position:        req.position,
            job_description: req.job_description,
            round:           req.round,
            scheduled_at:    req.scheduled_at,
            task_status:     InterviewTaskStatus::Pending,
            prep_materials:  PrepMaterials::default(),
            notes:           req.notes,
            trace_id:        None,
            is_deleted:      false,
            deleted_at:      None,
            created_at:      now,
            updated_at:      now,
        };

        self.repo.save(&plan).await
    }

    /// Create a new interview plan **and** generate AI prep materials.
    #[instrument(skip(self, req, prep_req))]
    pub async fn create_plan_with_prep(
        &self,
        req: CreateInterviewPlanRequest,
        prep_req: PrepGenerationRequest,
    ) -> Result<InterviewPlan, InterviewError> {
        Self::validate_create_request(&req)?;

        let materials = self.generate_materials(&prep_req).await?;

        let now = Timestamp::now();
        let plan = InterviewPlan {
            id:              InterviewId::new(),
            application_id:  req.application_id,
            title:           req.title,
            company:         req.company,
            position:        req.position,
            job_description: req.job_description,
            round:           req.round,
            scheduled_at:    req.scheduled_at,
            task_status:     InterviewTaskStatus::Pending,
            prep_materials:  materials,
            notes:           req.notes,
            trace_id:        None,
            is_deleted:      false,
            deleted_at:      None,
            created_at:      now,
            updated_at:      now,
        };

        self.repo.save(&plan).await
    }

    // -- Read ---------------------------------------------------------------

    /// Retrieve an interview plan by id.
    #[instrument(skip(self))]
    pub async fn get_plan(&self, id: InterviewId) -> Result<InterviewPlan, InterviewError> {
        self.repo.find_by_id(id).await?.ok_or_else(|| {
            NotFoundSnafu {
                id: id.into_inner(),
            }
            .build()
        })
    }

    /// List interview plans matching a filter.
    #[instrument(skip(self, filter))]
    pub async fn list_plans(
        &self,
        filter: &InterviewFilter,
    ) -> Result<Vec<InterviewPlan>, InterviewError> {
        self.repo.find_all(filter).await
    }

    /// List all interview plans for a specific application.
    #[instrument(skip(self))]
    pub async fn list_plans_for_application(
        &self,
        app_id: ApplicationId,
    ) -> Result<Vec<InterviewPlan>, InterviewError> {
        self.repo.find_by_application(app_id).await
    }

    // -- Update -------------------------------------------------------------

    /// Apply a partial update to an existing interview plan.
    #[instrument(skip(self, req))]
    pub async fn update_plan(
        &self,
        id: InterviewId,
        req: UpdateInterviewPlanRequest,
    ) -> Result<InterviewPlan, InterviewError> {
        let mut plan = self.get_plan(id).await?;

        if let Some(title) = req.title {
            plan.title = title;
        }
        if let Some(company) = req.company {
            plan.company = company;
        }
        if let Some(position) = req.position {
            plan.position = position;
        }
        if let Some(jd) = req.job_description {
            plan.job_description = jd;
        }
        if let Some(round) = req.round {
            plan.round = round;
        }
        if let Some(scheduled_at) = req.scheduled_at {
            plan.scheduled_at = scheduled_at;
        }
        if let Some(materials) = req.prep_materials {
            plan.prep_materials = materials;
        }
        if let Some(notes) = req.notes {
            plan.notes = notes;
        }

        plan.updated_at = Timestamp::now();
        self.repo.update(&plan).await
    }

    /// Transition the task status of an interview plan.
    #[instrument(skip(self))]
    pub async fn update_status(
        &self,
        id: InterviewId,
        new_status: InterviewTaskStatus,
    ) -> Result<InterviewPlan, InterviewError> {
        let mut plan = self.get_plan(id).await?;

        Self::validate_status_transition(plan.task_status, new_status)?;

        plan.task_status = new_status;
        plan.updated_at = Timestamp::now();

        self.repo.update(&plan).await
    }

    /// Re-generate prep materials for an existing plan using AI.
    #[instrument(skip(self, prep_req))]
    pub async fn regenerate_prep(
        &self,
        id: InterviewId,
        prep_req: PrepGenerationRequest,
    ) -> Result<InterviewPlan, InterviewError> {
        let mut plan = self.get_plan(id).await?;

        let materials = self.generate_materials(&prep_req).await?;

        plan.prep_materials = materials;
        plan.updated_at = Timestamp::now();

        self.repo.update(&plan).await
    }

    // -- Delete -------------------------------------------------------------

    /// Soft-delete an interview plan.
    #[instrument(skip(self))]
    pub async fn delete_plan(&self, id: InterviewId) -> Result<(), InterviewError> {
        // Verify it exists first.
        let _ = self.get_plan(id).await?;
        self.repo.delete(id).await
    }

    // -- Internal helpers ---------------------------------------------------

    /// Validate that a create request has the required fields.
    fn validate_create_request(req: &CreateInterviewPlanRequest) -> Result<(), InterviewError> {
        if req.title.trim().is_empty() {
            return Err(ValidationSnafu {
                reason: "title must not be empty".to_owned(),
            }
            .build());
        }
        if req.company.trim().is_empty() {
            return Err(ValidationSnafu {
                reason: "company must not be empty".to_owned(),
            }
            .build());
        }
        if req.position.trim().is_empty() {
            return Err(ValidationSnafu {
                reason: "position must not be empty".to_owned(),
            }
            .build());
        }
        Ok(())
    }

    /// Validate that a status transition is allowed.
    fn validate_status_transition(
        from: InterviewTaskStatus,
        to: InterviewTaskStatus,
    ) -> Result<(), InterviewError> {
        let allowed = matches!(
            (from, to),
            (
                InterviewTaskStatus::Pending,
                InterviewTaskStatus::InProgress
            ) | (InterviewTaskStatus::Pending, InterviewTaskStatus::Skipped)
                | (
                    InterviewTaskStatus::InProgress,
                    InterviewTaskStatus::Completed
                )
                | (
                    InterviewTaskStatus::InProgress,
                    InterviewTaskStatus::Skipped
                )
        );

        if allowed {
            Ok(())
        } else {
            Err(InvalidStatusTransitionSnafu {
                from: from.to_string(),
                to:   to.to_string(),
            }
            .build())
        }
    }

    /// Generate prep materials, failing if no generator is configured.
    async fn generate_materials(
        &self,
        prep_req: &PrepGenerationRequest,
    ) -> Result<PrepMaterials, InterviewError> {
        let generator = self.prep_generator.as_ref().ok_or_else(|| {
            PrepGenerationFailedSnafu {
                message: "no prep generator configured".to_owned(),
            }
            .build()
        })?;

        generator.generate_prep(prep_req).await
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use sqlx::postgres::PgPoolOptions;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;

    use super::*;
    use crate::{
        pg_repository::PgInterviewPlanRepository, prep_generator::MockPrepGenerator,
        types::InterviewRound,
    };

    // -- Helpers ------------------------------------------------------------

    const TEST_JOB_ID: &str = "00000000-0000-0000-0000-000000000001";
    const TEST_APP_ID: &str = "00000000-0000-0000-0000-000000000002";

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

        // Run migrations in order.
        let migrations: &[&str] = &[
            include_str!("../../../common/yunara-store/migrations/20260127000000_init.sql"),
            include_str!(
                "../../../common/yunara-store/migrations/20260208000000_domain_models.sql"
            ),
            include_str!(
                "../../../common/yunara-store/migrations/20260209000000_resume_version_mgmt.sql"
            ),
            include_str!(
                "../../../common/yunara-store/migrations/20260210000000_schema_alignment.sql"
            ),
            include_str!(
                "../../../common/yunara-store/migrations/20260211000000_notify_priority.sql"
            ),
        ];

        for sql in migrations {
            sqlx::raw_sql(sql).execute(&pool).await.unwrap();
        }

        // The scheduler migration references set_updated_at() but the
        // function was created as trigger_set_updated_at() in the domain
        // migration. Fix the reference before executing.
        let scheduler_sql = include_str!(
            "../../../common/yunara-store/migrations/20260211000001_scheduler_tables.sql"
        )
        .replace(
            "FUNCTION set_updated_at()",
            "FUNCTION trigger_set_updated_at()",
        );
        sqlx::raw_sql(&scheduler_sql).execute(&pool).await.unwrap();

        // Convert domain enum columns to SMALLINT codes.
        let domain_int_migrations: &[&str] = &[
            include_str!(
                "../../../common/yunara-store/migrations/20260212000000_application_int_enums.sql"
            ),
            include_str!(
                "../../../common/yunara-store/migrations/20260212000001_interview_int_enums.sql"
            ),
            include_str!(
                "../../../common/yunara-store/migrations/20260212000002_notify_int_enums.sql"
            ),
            include_str!(
                "../../../common/yunara-store/migrations/20260212000003_resume_int_enums.sql"
            ),
            include_str!(
                "../../../common/yunara-store/migrations/20260212000004_scheduler_int_enums.sql"
            ),
        ];
        for sql in domain_int_migrations {
            sqlx::raw_sql(sql).execute(&pool).await.unwrap();
        }

        // Insert job and application rows to satisfy FK constraints.
        sqlx::query(
            r#"INSERT INTO job (id, source_job_id, source_name, title, company)
               VALUES ($1, 'test-1', 'manual', 'SWE', 'TestCo')"#,
        )
        .bind(TEST_JOB_ID.parse::<uuid::Uuid>().unwrap())
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            r#"INSERT INTO application (id, job_id, channel, status)
               VALUES ($1, $2, 0, 0)"#,
        )
        .bind(TEST_APP_ID.parse::<uuid::Uuid>().unwrap())
        .bind(TEST_JOB_ID.parse::<uuid::Uuid>().unwrap())
        .execute(&pool)
        .await
        .unwrap();

        (pool, container)
    }

    async fn make_service(
        with_prep: bool,
    ) -> (InterviewService, testcontainers::ContainerAsync<Postgres>) {
        let (pool, container) = setup_pool().await;
        let repo = Arc::new(PgInterviewPlanRepository::new(pool));
        let prep: Option<Arc<dyn PrepGenerator>> = if with_prep {
            Some(Arc::new(MockPrepGenerator::new()))
        } else {
            None
        };
        (InterviewService::new(repo, prep), container)
    }

    fn sample_create_request() -> CreateInterviewPlanRequest {
        CreateInterviewPlanRequest {
            application_id:  ApplicationId::from(TEST_APP_ID.parse::<uuid::Uuid>().unwrap()),
            title:           "Technical Interview - Acme".into(),
            company:         "Acme Corp".into(),
            position:        "Senior Rust Engineer".into(),
            job_description: Some("Build distributed systems in Rust.".into()),
            round:           InterviewRound::Technical,
            scheduled_at:    None,
            notes:           None,
        }
    }

    fn sample_prep_request() -> PrepGenerationRequest {
        PrepGenerationRequest {
            company:         "Acme Corp".into(),
            position:        "Senior Rust Engineer".into(),
            job_description: "Build distributed systems in Rust.".into(),
            round:           InterviewRound::Technical,
            resume_content:  None,
            previous_rounds: vec![],
            email_context:   None,
        }
    }

    // -- Tests --------------------------------------------------------------

    #[tokio::test]
    async fn create_plan_succeeds() {
        let (svc, _container) = make_service(false).await;
        let req = sample_create_request();
        let plan = svc.create_plan(req).await.expect("create should succeed");

        assert_eq!(plan.company, "Acme Corp");
        assert_eq!(plan.task_status, InterviewTaskStatus::Pending);
        assert!(plan.prep_materials.knowledge_points.is_empty());
    }

    #[tokio::test]
    async fn create_plan_validates_empty_title() {
        let (svc, _container) = make_service(false).await;
        let mut req = sample_create_request();
        req.title = "  ".into();

        let err = svc.create_plan(req).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("title"), "error should mention title: {msg}");
    }

    #[tokio::test]
    async fn create_plan_with_prep_generates_materials() {
        let (svc, _container) = make_service(true).await;
        let req = sample_create_request();
        let prep_req = sample_prep_request();
        let plan = svc
            .create_plan_with_prep(req, prep_req)
            .await
            .expect("create_with_prep should succeed");

        assert!(
            !plan.prep_materials.knowledge_points.is_empty(),
            "should have generated knowledge points"
        );
        assert!(
            !plan.prep_materials.behavioral_questions.is_empty(),
            "should have generated behavioral questions"
        );
    }

    #[tokio::test]
    async fn create_plan_with_prep_fails_without_generator() {
        let (svc, _container) = make_service(false).await;
        let req = sample_create_request();
        let prep_req = sample_prep_request();
        let err = svc.create_plan_with_prep(req, prep_req).await.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("no prep generator"),
            "error should mention missing generator: {msg}"
        );
    }

    #[tokio::test]
    async fn get_plan_returns_not_found() {
        let (svc, _container) = make_service(false).await;
        let err = svc.get_plan(InterviewId::new()).await.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("not found"),
            "error should mention not found: {msg}"
        );
    }

    #[tokio::test]
    async fn update_plan_applies_changes() {
        let (svc, _container) = make_service(false).await;
        let plan = svc
            .create_plan(sample_create_request())
            .await
            .expect("create");

        let update = UpdateInterviewPlanRequest {
            title: Some("Updated Title".into()),
            ..Default::default()
        };
        let updated = svc.update_plan(plan.id, update).await.expect("update");
        assert_eq!(updated.title, "Updated Title");
        assert_eq!(updated.company, "Acme Corp");
    }

    #[tokio::test]
    async fn status_transition_valid() {
        let (svc, _container) = make_service(false).await;
        let plan = svc
            .create_plan(sample_create_request())
            .await
            .expect("create");

        let in_progress = svc
            .update_status(plan.id, InterviewTaskStatus::InProgress)
            .await
            .expect("pending -> in_progress");
        assert_eq!(in_progress.task_status, InterviewTaskStatus::InProgress);

        let completed = svc
            .update_status(plan.id, InterviewTaskStatus::Completed)
            .await
            .expect("in_progress -> completed");
        assert_eq!(completed.task_status, InterviewTaskStatus::Completed);
    }

    #[tokio::test]
    async fn status_transition_invalid() {
        let (svc, _container) = make_service(false).await;
        let plan = svc
            .create_plan(sample_create_request())
            .await
            .expect("create");

        // pending -> completed is not allowed
        let err = svc
            .update_status(plan.id, InterviewTaskStatus::Completed)
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("invalid status transition"),
            "should reject invalid transition: {msg}"
        );
    }

    #[tokio::test]
    async fn regenerate_prep_updates_materials() {
        let (svc, _container) = make_service(true).await;
        let plan = svc
            .create_plan(sample_create_request())
            .await
            .expect("create");

        assert!(
            plan.prep_materials.knowledge_points.is_empty(),
            "initially empty"
        );

        let regenerated = svc
            .regenerate_prep(plan.id, sample_prep_request())
            .await
            .expect("regenerate");

        assert!(
            !regenerated.prep_materials.knowledge_points.is_empty(),
            "should now have materials"
        );
    }

    #[tokio::test]
    async fn delete_plan_soft_deletes() {
        let (svc, _container) = make_service(false).await;
        let plan = svc
            .create_plan(sample_create_request())
            .await
            .expect("create");

        svc.delete_plan(plan.id).await.expect("delete");

        let err = svc.get_plan(plan.id).await.unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[tokio::test]
    async fn list_plans_for_application() {
        let (svc, _container) = make_service(false).await;
        let app_id = ApplicationId::from(TEST_APP_ID.parse::<uuid::Uuid>().unwrap());

        let mut req1 = sample_create_request();
        req1.application_id = app_id;
        req1.title = "Round 1".into();

        let mut req2 = sample_create_request();
        req2.application_id = app_id;
        req2.title = "Round 2".into();

        svc.create_plan(req1).await.expect("create 1");
        svc.create_plan(req2).await.expect("create 2");

        let plans = svc.list_plans_for_application(app_id).await.expect("list");
        assert_eq!(plans.len(), 2);
    }
}
