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
use job_domain_shared::id::ApplicationId;
use sqlx::PgPool;

use crate::{
    db_models,
    error::ApplicationError,
    types::{Application, ApplicationFilter, StatusChangeRecord},
};

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
        let store: db_models::Application = app.clone().into();

        let row = sqlx::query_as::<_, db_models::Application>(
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
        let row = sqlx::query_as::<_, db_models::Application>(
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

        let rows = sqlx::query_as::<_, db_models::Application>(&sql)
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
        let store: db_models::Application = app.clone().into();

        let row = sqlx::query_as::<_, db_models::Application>(
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
        let store: db_models::ApplicationStatusHistory = record.clone().into();

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
        let rows = sqlx::query_as::<_, db_models::ApplicationStatusHistory>(
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use job_domain_shared::id::{JobSourceId, ResumeId};

    use crate::types::ApplicationStatus;
    use sqlx::postgres::PgPoolOptions;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;

    use super::*;
    use crate::{
        repository::ApplicationRepository,
        types::{ApplicationChannel, ChangeSource, Priority},
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

        // Insert a job row to satisfy FK constraints
        sqlx::query(
            r#"INSERT INTO job (id, source_job_id, source_name, title, company)
               VALUES ('00000000-0000-0000-0000-000000000001', 'test-1', 'manual', 'SWE', 'TestCo')"#,
        )
        .execute(&pool)
        .await
        .unwrap();

        (pool, container)
    }

    fn make_app(job_id: uuid::Uuid) -> Application {
        let now = jiff::Timestamp::now();
        Application {
            id:           ApplicationId::new(),
            job_id:       JobSourceId::from(job_id),
            resume_id:    ResumeId::from(uuid::Uuid::nil()),
            channel:      ApplicationChannel::Direct,
            status:       ApplicationStatus::Draft,
            cover_letter: Some("My cover letter".into()),
            notes:        None,
            tags:         vec!["rust".into()],
            priority:     Priority::Medium,
            trace_id:     None,
            is_deleted:   false,
            submitted_at: None,
            created_at:   now,
            updated_at:   now,
        }
    }

    const TEST_JOB_ID: &str = "00000000-0000-0000-0000-000000000001";

    fn job_uuid() -> uuid::Uuid { TEST_JOB_ID.parse().unwrap() }

    #[tokio::test]
    async fn save_and_find_by_id() {
        let (pool, _container) = setup_pool().await;
        let repo = PgApplicationRepository::new(pool);

        let app = make_app(job_uuid());
        let saved = repo.save(&app).await.unwrap();
        assert_eq!(saved.id, app.id);

        let found = repo.find_by_id(app.id).await.unwrap().unwrap();
        assert_eq!(found.id, app.id);
        assert_eq!(found.channel, ApplicationChannel::Direct);
    }

    #[tokio::test]
    async fn update_application() {
        let (pool, _container) = setup_pool().await;
        let repo = PgApplicationRepository::new(pool);

        let app = make_app(job_uuid());
        repo.save(&app).await.unwrap();

        let mut updated_app = app.clone();
        updated_app.status = ApplicationStatus::Submitted;
        updated_app.cover_letter = Some("Updated CL".into());
        updated_app.submitted_at = Some(jiff::Timestamp::now());

        let updated = repo.update(&updated_app).await.unwrap();
        assert_eq!(updated.status, ApplicationStatus::Submitted);
        assert_eq!(updated.cover_letter.as_deref(), Some("Updated CL"));
    }

    #[tokio::test]
    async fn soft_delete() {
        let (pool, _container) = setup_pool().await;
        let repo = PgApplicationRepository::new(pool);

        let app = make_app(job_uuid());
        repo.save(&app).await.unwrap();

        repo.delete(app.id).await.unwrap();
        let found = repo.find_by_id(app.id).await.unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn soft_delete_not_found() {
        let (pool, _container) = setup_pool().await;
        let repo = PgApplicationRepository::new(pool);

        let result = repo.delete(ApplicationId::new()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn find_all_with_status_filter() {
        let (pool, _container) = setup_pool().await;
        let repo = PgApplicationRepository::new(pool);

        let app1 = make_app(job_uuid());
        repo.save(&app1).await.unwrap();

        let mut app2 = make_app(job_uuid());
        app2.status = ApplicationStatus::Submitted;
        app2.submitted_at = Some(jiff::Timestamp::now());
        repo.save(&app2).await.unwrap();

        let filter = ApplicationFilter {
            status: Some(ApplicationStatus::Draft),
            ..Default::default()
        };
        let results = repo.find_all(&filter).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, app1.id);
    }

    #[tokio::test]
    async fn find_all_with_tag_filter() {
        let (pool, _container) = setup_pool().await;
        let repo = PgApplicationRepository::new(pool);

        let mut app1 = make_app(job_uuid());
        app1.tags = vec!["rust".into(), "backend".into()];
        repo.save(&app1).await.unwrap();

        let mut app2 = make_app(job_uuid());
        app2.tags = vec!["python".into()];
        repo.save(&app2).await.unwrap();

        let filter = ApplicationFilter {
            tags: Some(vec!["rust".into()]),
            ..Default::default()
        };
        let results = repo.find_all(&filter).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, app1.id);
    }

    #[tokio::test]
    async fn save_and_get_status_history() {
        let (pool, _container) = setup_pool().await;
        let repo = PgApplicationRepository::new(pool);

        let app = make_app(job_uuid());
        repo.save(&app).await.unwrap();

        let record = StatusChangeRecord {
            id:             uuid::Uuid::new_v4(),
            application_id: app.id,
            from_status:    ApplicationStatus::Draft,
            to_status:      ApplicationStatus::Submitted,
            changed_by:     ChangeSource::Manual,
            note:           Some("Submitted the application".into()),
            created_at:     jiff::Timestamp::now(),
        };

        repo.save_status_change(&record).await.unwrap();

        let history = repo.get_status_history(app.id).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].from_status, ApplicationStatus::Draft);
        assert_eq!(history[0].to_status, ApplicationStatus::Submitted);
        assert_eq!(history[0].changed_by, ChangeSource::Manual);
    }
}
