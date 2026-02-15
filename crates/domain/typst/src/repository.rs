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

//! Repository trait for Typst project persistence.
//!
//! The trait is defined in the domain crate so that the service layer can
//! depend on it without pulling in any infrastructure code.
//!
//! File operations have been removed — files are now read/written directly
//! from the local filesystem via the `fs` module.

use uuid::Uuid;

use crate::{
    error::TypstError,
    types::{RenderResult, TypstProject},
};

/// Persistence contract for Typst projects and renders.
///
/// File storage has been moved to the local filesystem; see `crate::fs`.
#[async_trait::async_trait]
pub trait TypstRepository: Send + Sync {
    // -- Projects --

    /// Create a new project.
    async fn create_project(
        &self,
        name: &str,
        local_path: &str,
        main_file: &str,
        git_url: Option<&str>,
    ) -> Result<TypstProject, TypstError>;

    /// Update the git sync timestamp for a project.
    async fn update_git_synced(&self, id: Uuid) -> Result<TypstProject, TypstError>;

    /// Get a project by ID.
    async fn get_project(&self, id: Uuid) -> Result<Option<TypstProject>, TypstError>;

    /// List all projects (newest first).
    async fn list_projects(&self) -> Result<Vec<TypstProject>, TypstError>;

    /// Delete a project and all associated renders.
    async fn delete_project(&self, id: Uuid) -> Result<(), TypstError>;

    // -- Renders --

    /// Save a render result record.
    async fn create_render(
        &self,
        project_id: Uuid,
        pdf_object_key: &str,
        source_hash: &str,
        page_count: i32,
        file_size: i64,
    ) -> Result<RenderResult, TypstError>;

    /// Get a render by ID.
    async fn get_render(&self, id: Uuid) -> Result<Option<RenderResult>, TypstError>;

    /// List renders for a project (newest first).
    async fn list_renders(&self, project_id: Uuid) -> Result<Vec<RenderResult>, TypstError>;

    /// Find a render by source hash (for caching).
    async fn find_render_by_hash(
        &self,
        project_id: Uuid,
        source_hash: &str,
    ) -> Result<Option<RenderResult>, TypstError>;
}
