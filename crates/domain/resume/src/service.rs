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

//! Application-level service for resume version management.
//!
//! [`ResumeService`] orchestrates the domain logic -- content hashing,
//! deduplication checks, version tree construction, and diffing -- on
//! top of a [`ResumeRepository`] implementation.

use std::sync::Arc;

use tracing::instrument;
use uuid::Uuid;

use crate::{
    hash::content_hash,
    repository::ResumeRepository,
    types::{
        CreateResumeRequest, InvalidContentSnafu, NotFoundSnafu, Resume, ResumeDiff, ResumeError,
        ResumeFilter, ResumeSource, UpdateResumeRequest,
    },
    version::{ResumeVersionTree, compute_diff},
};

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

/// High-level service for resume CRUD, versioning, and diffing.
pub struct ResumeService<R: ResumeRepository> {
    repo: Arc<R>,
}

impl<R: ResumeRepository> Clone for ResumeService<R> {
    fn clone(&self) -> Self {
        Self {
            repo: self.repo.clone(),
        }
    }
}

impl<R: ResumeRepository> ResumeService<R> {
    /// Create a new service backed by the given repository.
    #[must_use]
    pub const fn new(repo: Arc<R>) -> Self { Self { repo } }

    // -- Baseline -----------------------------------------------------------

    /// Create a new baseline (source-of-truth) resume.
    ///
    /// A baseline resume has `source = Manual` and no parent.  Content
    /// is validated and hashed; if an identical hash already exists the
    /// call returns [`ResumeError::DuplicateContent`].
    #[instrument(skip(self, content))]
    pub async fn create_baseline(
        &self,
        title: String,
        content: String,
    ) -> Result<Resume, ResumeError> {
        Self::validate_content(&content)?;
        self.check_duplicate(&content).await?;

        let req = CreateResumeRequest {
            title,
            content,
            source: ResumeSource::Manual,
            parent_resume_id: None,
            target_job_id: None,
            customization_notes: None,
            tags: vec![],
        };

        self.repo.create(req).await
    }

    // -- Derive for job -----------------------------------------------------

    /// Derive a new resume version tailored for a specific job.
    ///
    /// The new resume is linked to `parent_id` and tagged with
    /// `target_job_id`.  The caller supplies the modified content and
    /// customization notes describing what changed.
    #[instrument(skip(self, content))]
    pub async fn derive_for_job(
        &self,
        parent_id: Uuid,
        job_id: Uuid,
        content: String,
        customization_notes: Option<String>,
    ) -> Result<Resume, ResumeError> {
        Self::validate_content(&content)?;

        // Verify parent exists.
        let parent = self
            .repo
            .get_by_id(parent_id)
            .await?
            .ok_or_else(|| NotFoundSnafu { id: parent_id }.build())?;

        self.check_duplicate(&content).await?;

        let req = CreateResumeRequest {
            title: format!("{} (for job {})", parent.title, job_id),
            content,
            source: ResumeSource::Optimized,
            parent_resume_id: Some(parent_id),
            target_job_id: Some(job_id),
            customization_notes,
            tags: parent.tags.clone(),
        };

        self.repo.create(req).await
    }

    // -- CRUD ---------------------------------------------------------------

    /// Create a resume with full control over all fields.
    #[instrument(skip(self, req))]
    pub async fn create(&self, req: CreateResumeRequest) -> Result<Resume, ResumeError> {
        Self::validate_content(&req.content)?;
        self.check_duplicate(&req.content).await?;
        self.repo.create(req).await
    }

    /// Get a resume by id.
    #[instrument(skip(self))]
    pub async fn get(&self, id: Uuid) -> Result<Option<Resume>, ResumeError> {
        self.repo.get_by_id(id).await
    }

    /// Update a resume.
    #[instrument(skip(self, req))]
    pub async fn update(&self, id: Uuid, req: UpdateResumeRequest) -> Result<Resume, ResumeError> {
        if let Some(ref content) = req.content {
            Self::validate_content(content)?;
            self.check_duplicate(content).await?;
        }
        self.repo.update(id, req).await
    }

    /// Soft-delete a resume.
    #[instrument(skip(self))]
    pub async fn delete(&self, id: Uuid) -> Result<(), ResumeError> {
        self.repo.soft_delete(id).await
    }

    /// List resumes matching the given filter.
    #[instrument(skip(self))]
    pub async fn list(&self, filter: ResumeFilter) -> Result<Vec<Resume>, ResumeError> {
        self.repo.list(filter).await
    }

    /// Get the current baseline resume.
    #[instrument(skip(self))]
    pub async fn get_baseline(&self) -> Result<Option<Resume>, ResumeError> {
        self.repo.get_baseline().await
    }

    // -- Versioning ---------------------------------------------------------

    /// Compute a line-level diff between two resume versions.
    #[allow(clippy::similar_names)]
    #[instrument(skip(self))]
    pub async fn get_diff(
        &self,
        resume_a_id: Uuid,
        resume_b_id: Uuid,
    ) -> Result<ResumeDiff, ResumeError> {
        let a = self
            .repo
            .get_by_id(resume_a_id)
            .await?
            .ok_or_else(|| NotFoundSnafu { id: resume_a_id }.build())?;

        let b = self
            .repo
            .get_by_id(resume_b_id)
            .await?
            .ok_or_else(|| NotFoundSnafu { id: resume_b_id }.build())?;

        let text_a = a.content.as_deref().unwrap_or("");
        let text_b = b.content.as_deref().unwrap_or("");

        Ok(compute_diff(resume_a_id, resume_b_id, text_a, text_b))
    }

    /// Build the full derivation tree for a resume by walking its parent
    /// chain.
    #[instrument(skip(self))]
    pub async fn get_version_tree(
        &self,
        resume_id: Uuid,
    ) -> Result<ResumeVersionTree, ResumeError> {
        let history = self.repo.get_version_history(resume_id).await?;
        Ok(ResumeVersionTree::from_history(&history))
    }

    /// Get all direct children of a resume.
    #[instrument(skip(self))]
    pub async fn get_children(&self, parent_id: Uuid) -> Result<Vec<Resume>, ResumeError> {
        self.repo.get_children(parent_id).await
    }

    // -- Internal helpers ---------------------------------------------------

    /// Validate that resume content is non-empty.
    fn validate_content(content: &str) -> Result<(), ResumeError> {
        if content.trim().is_empty() {
            return Err(InvalidContentSnafu {
                reason: "content must not be empty".to_owned(),
            }
            .build());
        }
        Ok(())
    }

    /// Check for duplicate content by hash and return an error if found.
    async fn check_duplicate(&self, content: &str) -> Result<(), ResumeError> {
        let hash = content_hash(content);
        if self.repo.find_by_content_hash(&hash).await?.is_some() {
            return Err(crate::types::DuplicateContentSnafu { content_hash: hash }.build());
        }
        Ok(())
    }
}
