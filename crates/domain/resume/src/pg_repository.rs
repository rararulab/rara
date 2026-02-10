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
use jiff::{Zoned, tz::TimeZone};
use job_domain_shared::convert::timestamp_to_chrono;
use job_model::resume::Resume as StoreResume;
use sqlx::PgPool;
use uuid::Uuid;

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
