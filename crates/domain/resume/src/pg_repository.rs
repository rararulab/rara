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

//! PostgreSQL-backed implementation of [`crate::repository::ResumeRepository`].

use std::fmt::Write;

use async_trait::async_trait;
use chrono::{DateTime, TimeZone as _, Utc};
use jiff::{Zoned, tz::TimeZone};
use sqlx::PgPool;
use uuid::Uuid;

use job_model::resume::Resume as StoreResume;

use crate::{
    hash::content_hash,
    types::{
        CreateResumeRequest, Resume, ResumeError, ResumeFilter, ResumeSource, UpdateResumeRequest,
    },
};

/// PostgreSQL implementation of the resume repository.
pub struct PgResumeRepository {
    pool: PgPool,
}

impl PgResumeRepository {
    /// Create a new repository backed by the given connection pool.
    #[must_use]
    pub fn new(pool: PgPool) -> Self { Self { pool } }
}

/// Map a `sqlx::Error` into a `ResumeError::Storage`.
fn map_err(e: sqlx::Error) -> ResumeError {
    ResumeError::Storage {
        message: e.to_string(),
    }
}

fn timestamp_to_chrono(ts: jiff::Timestamp) -> DateTime<Utc> {
    let mut second = ts.as_second();
    let mut nanosecond = ts.subsec_nanosecond();
    if nanosecond < 0 {
        second = second.saturating_sub(1);
        nanosecond = nanosecond.saturating_add(1_000_000_000);
    }

    Utc.timestamp_opt(second, nanosecond as u32)
        .single()
        .expect("jiff Timestamp fits in chrono DateTime<Utc>")
}

#[async_trait]
impl crate::repository::ResumeRepository for PgResumeRepository {
    async fn create(&self, req: CreateResumeRequest) -> Result<Resume, ResumeError> {
        let id = Uuid::new_v4();
        let hash = content_hash(&req.content);
        let version_tag = format!(
            "v{}",
            Zoned::now()
                .with_time_zone(TimeZone::UTC)
                .strftime("%Y%m%d%H%M%S")
        );
        let source_code = req.source as u8 as i16;

        let row = sqlx::query_as::<_, StoreResume>(
            r#"INSERT INTO resume (id, title, version_tag, content_hash, source, content,
                   parent_resume_id, target_job_id, customization_notes, tags)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
               RETURNING *"#,
        )
        .bind(id)
        .bind(&req.title)
        .bind(&version_tag)
        .bind(&hash)
        .bind(source_code)
        .bind(&req.content)
        .bind(req.parent_resume_id)
        .bind(req.target_job_id)
        .bind(&req.customization_notes)
        .bind(&req.tags)
        .fetch_one(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(row.into())
    }

    async fn get_by_id(&self, id: Uuid) -> Result<Option<Resume>, ResumeError> {
        let row = sqlx::query_as::<_, StoreResume>(
            "SELECT * FROM resume WHERE id = $1 AND is_deleted = FALSE",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(row.map(Into::into))
    }

    async fn update(&self, id: Uuid, req: UpdateResumeRequest) -> Result<Resume, ResumeError> {
        // Fetch current row first to merge partial updates.
        let current = sqlx::query_as::<_, StoreResume>(
            "SELECT * FROM resume WHERE id = $1 AND is_deleted = FALSE",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)?
        .ok_or(ResumeError::NotFound { id })?;

        let title = req.title.unwrap_or(current.title);
        let content = req.content.or(current.content.clone());
        let source = req.source.map(|s| s as u8 as i16).unwrap_or(current.source);
        let target_job_id = match req.target_job_id {
            Some(v) => v,
            None => current.target_job_id,
        };
        let customization_notes = match req.customization_notes {
            Some(v) => v,
            None => current.customization_notes,
        };
        let tags = req.tags.unwrap_or(current.tags);

        // Recompute content hash if content changed.
        let hash = content
            .as_deref()
            .map(content_hash)
            .unwrap_or(current.content_hash);

        let row = sqlx::query_as::<_, StoreResume>(
            r#"UPDATE resume
               SET title = $2, content = $3, content_hash = $4, source = $5,
                   target_job_id = $6, customization_notes = $7, tags = $8
               WHERE id = $1 AND is_deleted = FALSE
               RETURNING *"#,
        )
        .bind(id)
        .bind(&title)
        .bind(&content)
        .bind(&hash)
        .bind(&source)
        .bind(target_job_id)
        .bind(&customization_notes)
        .bind(&tags)
        .fetch_one(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(row.into())
    }

    async fn soft_delete(&self, id: Uuid) -> Result<(), ResumeError> {
        let result = sqlx::query(
            "UPDATE resume SET is_deleted = TRUE, deleted_at = now() WHERE id = $1 AND is_deleted \
             = FALSE",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;

        if result.rows_affected() == 0 {
            return Err(ResumeError::NotFound { id });
        }
        Ok(())
    }

    async fn list(&self, filter: ResumeFilter) -> Result<Vec<Resume>, ResumeError> {
        let mut sql = String::from("SELECT * FROM resume WHERE is_deleted = FALSE");
        let mut param_idx = 1u32;

        if let Some(ref source) = filter.source {
            let source_code = *source as u8 as i16;
            let _ = write!(sql, " AND source = {source_code}");
        }

        if let Some(has_parent) = filter.has_parent {
            if has_parent {
                sql.push_str(" AND parent_resume_id IS NOT NULL");
            } else {
                sql.push_str(" AND parent_resume_id IS NULL");
            }
        }

        if filter.target_job_id.is_some() {
            param_idx += 1;
            let _ = write!(sql, " AND target_job_id = ${param_idx}");
        }

        if filter.created_after.is_some() {
            param_idx += 1;
            let _ = write!(sql, " AND created_at >= ${param_idx}");
        }

        if filter.created_before.is_some() {
            param_idx += 1;
            let _ = write!(sql, " AND created_at <= ${param_idx}");
        }

        sql.push_str(" ORDER BY created_at DESC");

        let mut query = sqlx::query_as::<_, StoreResume>(&sql);

        if let Some(target_job_id) = filter.target_job_id {
            query = query.bind(target_job_id);
        }

        if let Some(created_after) = filter.created_after {
            query = query.bind(timestamp_to_chrono(created_after));
        }

        if let Some(created_before) = filter.created_before {
            query = query.bind(timestamp_to_chrono(created_before));
        }

        let rows = query.fetch_all(&self.pool).await.map_err(map_err)?;
        let mut results: Vec<Resume> = rows.into_iter().map(Into::into).collect();

        // Apply tag filter in-memory.
        if let Some(ref required_tags) = filter.tags {
            results.retain(|r| required_tags.iter().all(|t| r.tags.contains(t)));
        }

        Ok(results)
    }

    async fn get_baseline(&self) -> Result<Option<Resume>, ResumeError> {
        let manual_source = ResumeSource::Manual as u8 as i16;
        let row = sqlx::query_as::<_, StoreResume>(
            r#"SELECT * FROM resume
               WHERE source = $1
                 AND parent_resume_id IS NULL
                 AND is_deleted = FALSE
               ORDER BY created_at DESC
               LIMIT 1"#,
        )
        .bind(manual_source)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(row.map(Into::into))
    }

    async fn get_children(&self, parent_id: Uuid) -> Result<Vec<Resume>, ResumeError> {
        let rows = sqlx::query_as::<_, StoreResume>(
            "SELECT * FROM resume WHERE parent_resume_id = $1 AND is_deleted = FALSE ORDER BY \
             created_at",
        )
        .bind(parent_id)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn get_version_history(&self, resume_id: Uuid) -> Result<Vec<Resume>, ResumeError> {
        let rows = sqlx::query_as::<_, StoreResume>(
            r#"WITH RECURSIVE ancestry AS (
                   SELECT * FROM resume WHERE id = $1 AND is_deleted = FALSE
                   UNION ALL
                   SELECT r.* FROM resume r
                   JOIN ancestry a ON r.id = a.parent_resume_id
                   WHERE r.is_deleted = FALSE
               )
               SELECT * FROM ancestry ORDER BY created_at ASC"#,
        )
        .bind(resume_id)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn find_by_content_hash(
        &self,
        content_hash: &str,
    ) -> Result<Option<Resume>, ResumeError> {
        let row = sqlx::query_as::<_, StoreResume>(
            "SELECT * FROM resume WHERE content_hash = $1 AND is_deleted = FALSE",
        )
        .bind(content_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(row.map(Into::into))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use sqlx::postgres::PgPoolOptions;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;

    use super::*;
    use crate::repository::ResumeRepository;

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

        (pool, container)
    }

    #[tokio::test]
    async fn create_and_get_by_id() {
        let (pool, _container) = setup_pool().await;
        let repo = PgResumeRepository::new(pool);

        let req = CreateResumeRequest {
            title:               "Backend Engineer v1".into(),
            content:             "My resume content".into(),
            source:              ResumeSource::Manual,
            parent_resume_id:    None,
            target_job_id:       None,
            customization_notes: None,
            tags:                vec!["rust".into(), "backend".into()],
        };

        let created = repo.create(req).await.unwrap();
        assert_eq!(created.title, "Backend Engineer v1");
        assert_eq!(created.source, ResumeSource::Manual);
        assert_eq!(created.tags, vec!["rust", "backend"]);
        assert!(!created.is_deleted);

        let fetched = repo.get_by_id(created.id).await.unwrap().unwrap();
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.title, "Backend Engineer v1");
    }

    #[tokio::test]
    async fn update_resume() {
        let (pool, _container) = setup_pool().await;
        let repo = PgResumeRepository::new(pool);

        let req = CreateResumeRequest {
            title:               "Original Title".into(),
            content:             "Original content".into(),
            source:              ResumeSource::Manual,
            parent_resume_id:    None,
            target_job_id:       None,
            customization_notes: None,
            tags:                vec![],
        };

        let created = repo.create(req).await.unwrap();

        let update = UpdateResumeRequest {
            title: Some("Updated Title".into()),
            content: Some("Updated content".into()),
            tags: Some(vec!["new-tag".into()]),
            ..Default::default()
        };

        let updated = repo.update(created.id, update).await.unwrap();
        assert_eq!(updated.title, "Updated Title");
        assert_eq!(updated.content.as_deref(), Some("Updated content"));
        assert_eq!(updated.tags, vec!["new-tag"]);
    }

    #[tokio::test]
    async fn soft_delete() {
        let (pool, _container) = setup_pool().await;
        let repo = PgResumeRepository::new(pool);

        let req = CreateResumeRequest {
            title:               "To Delete".into(),
            content:             "content".into(),
            source:              ResumeSource::Manual,
            parent_resume_id:    None,
            target_job_id:       None,
            customization_notes: None,
            tags:                vec![],
        };

        let created = repo.create(req).await.unwrap();
        repo.soft_delete(created.id).await.unwrap();

        let fetched = repo.get_by_id(created.id).await.unwrap();
        assert!(fetched.is_none());
    }

    #[tokio::test]
    async fn soft_delete_not_found() {
        let (pool, _container) = setup_pool().await;
        let repo = PgResumeRepository::new(pool);

        let result = repo.soft_delete(Uuid::new_v4()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn list_with_source_filter() {
        let (pool, _container) = setup_pool().await;
        let repo = PgResumeRepository::new(pool);

        // Create manual resume
        repo.create(CreateResumeRequest {
            title:               "Manual".into(),
            content:             "manual content".into(),
            source:              ResumeSource::Manual,
            parent_resume_id:    None,
            target_job_id:       None,
            customization_notes: None,
            tags:                vec![],
        })
        .await
        .unwrap();

        // Create AI resume
        repo.create(CreateResumeRequest {
            title:               "AI".into(),
            content:             "ai content".into(),
            source:              ResumeSource::AiGenerated,
            parent_resume_id:    None,
            target_job_id:       None,
            customization_notes: None,
            tags:                vec![],
        })
        .await
        .unwrap();

        let filter = ResumeFilter {
            source: Some(ResumeSource::Manual),
            ..Default::default()
        };
        let results = repo.list(filter).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Manual");
    }

    #[tokio::test]
    async fn list_with_tag_filter() {
        let (pool, _container) = setup_pool().await;
        let repo = PgResumeRepository::new(pool);

        repo.create(CreateResumeRequest {
            title:               "Tagged".into(),
            content:             "tagged content".into(),
            source:              ResumeSource::Manual,
            parent_resume_id:    None,
            target_job_id:       None,
            customization_notes: None,
            tags:                vec!["rust".into(), "backend".into()],
        })
        .await
        .unwrap();

        repo.create(CreateResumeRequest {
            title:               "Untagged".into(),
            content:             "untagged content".into(),
            source:              ResumeSource::Manual,
            parent_resume_id:    None,
            target_job_id:       None,
            customization_notes: None,
            tags:                vec!["python".into()],
        })
        .await
        .unwrap();

        let filter = ResumeFilter {
            tags: Some(vec!["rust".into()]),
            ..Default::default()
        };
        let results = repo.list(filter).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Tagged");
    }

    #[tokio::test]
    async fn get_baseline() {
        let (pool, _container) = setup_pool().await;
        let repo = PgResumeRepository::new(pool);

        // No baseline initially
        let baseline = repo.get_baseline().await.unwrap();
        assert!(baseline.is_none());

        // Create a baseline (manual, no parent)
        let created = repo
            .create(CreateResumeRequest {
                title:               "Baseline".into(),
                content:             "baseline content".into(),
                source:              ResumeSource::Manual,
                parent_resume_id:    None,
                target_job_id:       None,
                customization_notes: None,
                tags:                vec![],
            })
            .await
            .unwrap();

        let baseline = repo.get_baseline().await.unwrap().unwrap();
        assert_eq!(baseline.id, created.id);
    }

    #[tokio::test]
    async fn get_children_and_version_history() {
        let (pool, _container) = setup_pool().await;
        let repo = PgResumeRepository::new(pool);

        let parent = repo
            .create(CreateResumeRequest {
                title:               "Parent".into(),
                content:             "parent content".into(),
                source:              ResumeSource::Manual,
                parent_resume_id:    None,
                target_job_id:       None,
                customization_notes: None,
                tags:                vec![],
            })
            .await
            .unwrap();

        let child = repo
            .create(CreateResumeRequest {
                title:               "Child".into(),
                content:             "child content".into(),
                source:              ResumeSource::AiGenerated,
                parent_resume_id:    Some(parent.id),
                target_job_id:       None,
                customization_notes: Some("Tailored for SWE role".into()),
                tags:                vec![],
            })
            .await
            .unwrap();

        let children = repo.get_children(parent.id).await.unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].id, child.id);

        let history = repo.get_version_history(child.id).await.unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].id, parent.id);
        assert_eq!(history[1].id, child.id);
    }

    #[tokio::test]
    async fn find_by_content_hash() {
        let (pool, _container) = setup_pool().await;
        let repo = PgResumeRepository::new(pool);

        let created = repo
            .create(CreateResumeRequest {
                title:               "Hashable".into(),
                content:             "unique content".into(),
                source:              ResumeSource::Manual,
                parent_resume_id:    None,
                target_job_id:       None,
                customization_notes: None,
                tags:                vec![],
            })
            .await
            .unwrap();

        let found = repo
            .find_by_content_hash(&created.content_hash)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, created.id);

        let not_found = repo.find_by_content_hash("nonexistent_hash").await.unwrap();
        assert!(not_found.is_none());
    }
}
