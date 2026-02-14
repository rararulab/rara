//! Repository trait for Typst project persistence.
//!
//! The trait is defined in the domain crate so that the service layer can
//! depend on it without pulling in any infrastructure code.

use uuid::Uuid;

use crate::{
    error::TypstError,
    types::{RenderResult, TypstFile, TypstProject},
};

/// Persistence contract for Typst projects, files, and renders.
#[async_trait::async_trait]
pub trait TypstRepository: Send + Sync {
    // -- Projects --

    /// Create a new project.
    async fn create_project(
        &self,
        name: &str,
        description: Option<&str>,
        main_file: &str,
        resume_id: Option<Uuid>,
    ) -> Result<TypstProject, TypstError>;

    /// Get a project by ID.
    async fn get_project(&self, id: Uuid) -> Result<Option<TypstProject>, TypstError>;

    /// List all projects (newest first).
    async fn list_projects(&self) -> Result<Vec<TypstProject>, TypstError>;

    /// Delete a project and all associated files and renders.
    async fn delete_project(&self, id: Uuid) -> Result<(), TypstError>;

    // -- Files --

    /// Create a file in a project.
    async fn create_file(
        &self,
        project_id: Uuid,
        path: &str,
        content: &str,
    ) -> Result<TypstFile, TypstError>;

    /// Get a file by project ID and path.
    async fn get_file(
        &self,
        project_id: Uuid,
        path: &str,
    ) -> Result<Option<TypstFile>, TypstError>;

    /// List all files in a project.
    async fn list_files(&self, project_id: Uuid) -> Result<Vec<TypstFile>, TypstError>;

    /// Update a file's content.
    async fn update_file(
        &self,
        project_id: Uuid,
        path: &str,
        content: &str,
    ) -> Result<TypstFile, TypstError>;

    /// Delete a file by project ID and path.
    async fn delete_file(&self, project_id: Uuid, path: &str) -> Result<(), TypstError>;

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
