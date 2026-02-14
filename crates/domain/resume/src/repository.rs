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

//! Repository trait for resume persistence.
//!
//! The trait lives in the domain crate so that the service layer can
//! depend on it without pulling in any infrastructure code.
//! Implementations are expected to live in the infrastructure/store
//! layer.

use uuid::Uuid;

use crate::types::{CreateResumeRequest, Resume, ResumeError, ResumeFilter, UpdateResumeRequest};

/// Persistence contract for resume documents.
#[async_trait::async_trait]
pub trait ResumeRepository: Send + Sync {
    /// Persist a new resume version.
    async fn create(&self, req: CreateResumeRequest) -> Result<Resume, ResumeError>;

    /// Retrieve a single resume by its primary key.
    ///
    /// Returns `None` when the id does not exist or the row is
    /// soft-deleted.
    async fn get_by_id(&self, id: Uuid) -> Result<Option<Resume>, ResumeError>;

    /// Apply a partial update to an existing resume.
    async fn update(&self, id: Uuid, req: UpdateResumeRequest) -> Result<Resume, ResumeError>;

    /// Soft-delete a resume (set `is_deleted = true`).
    async fn soft_delete(&self, id: Uuid) -> Result<(), ResumeError>;

    /// List resumes matching the given filter criteria.
    async fn list(&self, filter: ResumeFilter) -> Result<Vec<Resume>, ResumeError>;

    /// Return the baseline (root) resume -- the one with `source =
    /// Manual` and no parent.
    ///
    /// If multiple baselines exist the most recently created one is
    /// returned.
    async fn get_baseline(&self) -> Result<Option<Resume>, ResumeError>;

    /// Retrieve all direct children of the given parent resume.
    async fn get_children(&self, parent_id: Uuid) -> Result<Vec<Resume>, ResumeError>;

    /// Walk the parent chain from the given resume back to the root and
    /// return the full ancestry, ordered oldest-first.
    async fn get_version_history(&self, resume_id: Uuid) -> Result<Vec<Resume>, ResumeError>;

    /// Check whether a resume with the given content hash already exists.
    async fn find_by_content_hash(&self, content_hash: &str)
    -> Result<Option<Resume>, ResumeError>;

    /// Create a resume record associated with an uploaded PDF file.
    async fn create_with_pdf(
        &self,
        req: CreateResumeRequest,
        pdf_object_key: String,
        pdf_file_size: i64,
    ) -> Result<Resume, ResumeError>;
}
