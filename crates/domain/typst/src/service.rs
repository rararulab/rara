//! Application-level service for Typst project management and compilation.

use std::{collections::HashMap, sync::Arc};

use opendal::Operator;
use sha2::{Digest, Sha256};
use tracing::instrument;
use uuid::Uuid;

use crate::{
    compiler,
    error::{TypstError, map_storage_err},
    git::GitImporter,
    repository::TypstRepository,
    types::{ImportGitRequest, RenderResult, TypstFile, TypstProject},
};

/// High-level service for Typst project CRUD and compilation.
pub struct TypstService {
    repo:         Arc<dyn TypstRepository>,
    object_store: Operator,
}

impl Clone for TypstService {
    fn clone(&self) -> Self {
        Self {
            repo:         self.repo.clone(),
            object_store: self.object_store.clone(),
        }
    }
}

impl TypstService {
    /// Create a new service backed by the given repository and object store.
    #[must_use]
    pub fn new(repo: Arc<dyn TypstRepository>, object_store: Operator) -> Self {
        Self { repo, object_store }
    }

    // -- Projects --

    /// Create a new Typst project.
    #[instrument(skip(self))]
    pub async fn create_project(
        &self,
        name: String,
        description: Option<String>,
        main_file: Option<String>,
        resume_id: Option<Uuid>,
    ) -> Result<TypstProject, TypstError> {
        if name.trim().is_empty() {
            return Err(TypstError::InvalidRequest {
                message: "project name must not be empty".to_owned(),
            });
        }
        let main_file = main_file.unwrap_or_else(|| "main.typ".to_owned());
        self.repo
            .create_project(&name, description.as_deref(), &main_file, resume_id, None)
            .await
    }

    /// Get a project by ID.
    #[instrument(skip(self))]
    pub async fn get_project(&self, id: Uuid) -> Result<TypstProject, TypstError> {
        self.repo
            .get_project(id)
            .await?
            .ok_or(TypstError::ProjectNotFound { id })
    }

    /// List all projects.
    #[instrument(skip(self))]
    pub async fn list_projects(&self) -> Result<Vec<TypstProject>, TypstError> {
        self.repo.list_projects().await
    }

    /// Delete a project and all associated data.
    #[instrument(skip(self))]
    pub async fn delete_project(&self, id: Uuid) -> Result<(), TypstError> {
        // Clean up S3 objects for all renders.
        let renders = self.repo.list_renders(id).await?;
        for render in &renders {
            if let Err(e) = self
                .object_store
                .delete(&render.pdf_object_key)
                .await
            {
                tracing::warn!(
                    key = %render.pdf_object_key,
                    error = %e,
                    "failed to delete render PDF from object store"
                );
            }
        }
        self.repo.delete_project(id).await
    }

    // -- Files --

    /// Create a file in a project.
    #[instrument(skip(self, content))]
    pub async fn create_file(
        &self,
        project_id: Uuid,
        path: String,
        content: String,
    ) -> Result<TypstFile, TypstError> {
        // Ensure the project exists.
        self.repo
            .get_project(project_id)
            .await?
            .ok_or(TypstError::ProjectNotFound { id: project_id })?;

        validate_file_path(&path)?;
        self.repo.create_file(project_id, &path, &content).await
    }

    /// Get a file's content.
    #[instrument(skip(self))]
    pub async fn get_file(
        &self,
        project_id: Uuid,
        path: &str,
    ) -> Result<TypstFile, TypstError> {
        self.repo
            .get_file(project_id, path)
            .await?
            .ok_or_else(|| TypstError::FileNotFound {
                project_id,
                path: path.to_owned(),
            })
    }

    /// List all files in a project.
    #[instrument(skip(self))]
    pub async fn list_files(&self, project_id: Uuid) -> Result<Vec<TypstFile>, TypstError> {
        self.repo.list_files(project_id).await
    }

    /// Update a file's content.
    #[instrument(skip(self, content))]
    pub async fn update_file(
        &self,
        project_id: Uuid,
        path: &str,
        content: String,
    ) -> Result<TypstFile, TypstError> {
        self.repo.update_file(project_id, path, &content).await
    }

    /// Delete a file.
    #[instrument(skip(self))]
    pub async fn delete_file(
        &self,
        project_id: Uuid,
        path: &str,
    ) -> Result<(), TypstError> {
        self.repo.delete_file(project_id, path).await
    }

    // -- Git integration --

    /// Import a Typst project from a Git repository.
    ///
    /// Clones the repo, scans for supported files, creates a project, and
    /// batch-inserts all discovered files.
    #[instrument(skip(self))]
    pub async fn import_from_git(
        &self,
        request: ImportGitRequest,
    ) -> Result<TypstProject, TypstError> {
        let importer = GitImporter;
        let files = importer.import_from_url(&request.url).await?;

        if files.is_empty() {
            return Err(TypstError::InvalidRequest {
                message: "no supported files found in repository".to_owned(),
            });
        }

        // Auto-detect main file: prefer `main.typ`, otherwise the first `.typ` file.
        let main_file = files
            .iter()
            .find(|f| f.path == "main.typ")
            .or_else(|| files.iter().find(|f| f.path.ends_with(".typ")))
            .map(|f| f.path.clone())
            .unwrap_or_else(|| "main.typ".to_owned());

        // Derive project name from request or URL.
        let name = request.name.unwrap_or_else(|| {
            request
                .url
                .rsplit('/')
                .next()
                .unwrap_or("imported-project")
                .trim_end_matches(".git")
                .to_owned()
        });

        let project = self
            .repo
            .create_project(&name, None, &main_file, None, Some(&request.url))
            .await?;

        // Batch-insert files.
        for file in &files {
            self.repo
                .create_file(project.id, &file.path, &file.content)
                .await?;
        }

        // Update sync timestamp.
        let project = self.repo.update_git_synced(project.id).await?;

        tracing::info!(
            project_id = %project.id,
            file_count = files.len(),
            git_url = %request.url,
            "imported project from git"
        );

        Ok(project)
    }

    /// Sync a Git-backed project by re-cloning and replacing all files.
    #[instrument(skip(self))]
    pub async fn sync_git(&self, project_id: Uuid) -> Result<TypstProject, TypstError> {
        let project = self
            .repo
            .get_project(project_id)
            .await?
            .ok_or(TypstError::ProjectNotFound { id: project_id })?;

        let git_url = project.git_url.as_deref().ok_or(TypstError::NotGitProject)?;

        let importer = GitImporter;
        let files = importer.sync(git_url).await?;

        // Full replace: delete old files, insert new ones.
        self.repo.delete_all_files(project_id).await?;

        for file in &files {
            self.repo
                .create_file(project_id, &file.path, &file.content)
                .await?;
        }

        // Update sync timestamp.
        let project = self.repo.update_git_synced(project_id).await?;

        tracing::info!(
            project_id = %project_id,
            file_count = files.len(),
            "synced project from git"
        );

        Ok(project)
    }

    // -- Compilation --

    /// Compile a project to PDF.
    ///
    /// If a cached render exists with the same source hash, it is returned
    /// instead of recompiling.
    #[instrument(skip(self))]
    pub async fn compile(
        &self,
        project_id: Uuid,
        main_file_override: Option<String>,
    ) -> Result<RenderResult, TypstError> {
        let project = self
            .repo
            .get_project(project_id)
            .await?
            .ok_or(TypstError::ProjectNotFound { id: project_id })?;

        let main_file = main_file_override.unwrap_or(project.main_file);

        // Gather all files.
        let files = self.repo.list_files(project_id).await?;
        if files.is_empty() {
            return Err(TypstError::InvalidRequest {
                message: "project has no files".to_owned(),
            });
        }

        let file_map: HashMap<String, String> = files
            .iter()
            .map(|f| (f.path.clone(), f.content.clone()))
            .collect();

        // Compute source hash for caching.
        let source_hash = compute_source_hash(&file_map, &main_file);

        // Check cache.
        if let Some(cached) = self
            .repo
            .find_render_by_hash(project_id, &source_hash)
            .await?
        {
            tracing::info!(
                project_id = %project_id,
                source_hash = %source_hash,
                render_id = %cached.id,
                "returning cached render"
            );
            return Ok(cached);
        }

        // Compile.
        let (pdf_bytes, page_count) = compiler::compile(&file_map, &main_file)?;
        let file_size = pdf_bytes.len() as i64;

        // Upload to S3.
        let object_key = format!("typst/renders/{project_id}/{}.pdf", Uuid::new_v4());
        self.object_store
            .write(&object_key, pdf_bytes)
            .await
            .map_err(map_storage_err)?;

        // Save render record.
        let render = self
            .repo
            .create_render(
                project_id,
                &object_key,
                &source_hash,
                page_count as i32,
                file_size,
            )
            .await?;

        tracing::info!(
            project_id = %project_id,
            render_id = %render.id,
            page_count,
            file_size,
            "compilation successful"
        );

        Ok(render)
    }

    /// List render history for a project.
    #[instrument(skip(self))]
    pub async fn list_renders(
        &self,
        project_id: Uuid,
    ) -> Result<Vec<RenderResult>, TypstError> {
        self.repo.list_renders(project_id).await
    }

    /// Get a render's PDF bytes from object storage.
    #[instrument(skip(self))]
    pub async fn get_render_pdf(&self, render_id: Uuid) -> Result<(Vec<u8>, String), TypstError> {
        let render = self
            .repo
            .get_render(render_id)
            .await?
            .ok_or(TypstError::RenderNotFound { id: render_id })?;

        let pdf_bytes = self
            .object_store
            .read(&render.pdf_object_key)
            .await
            .map_err(map_storage_err)?
            .to_vec();

        Ok((pdf_bytes, render.pdf_object_key))
    }
}

/// Validate that a file path is relative and doesn't contain path traversal.
fn validate_file_path(path: &str) -> Result<(), TypstError> {
    if path.trim().is_empty() {
        return Err(TypstError::InvalidRequest {
            message: "file path must not be empty".to_owned(),
        });
    }
    if path.starts_with('/') || path.contains("..") {
        return Err(TypstError::InvalidRequest {
            message: "file path must be relative and must not contain '..'".to_owned(),
        });
    }
    Ok(())
}

/// Compute a deterministic SHA-256 hash over all source files and the main
/// file path. Files are sorted by path to ensure reproducibility.
fn compute_source_hash(files: &HashMap<String, String>, main_file: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(main_file.as_bytes());
    hasher.update(b"\0");

    let mut paths: Vec<&String> = files.keys().collect();
    paths.sort();

    for path in paths {
        hasher.update(path.as_bytes());
        hasher.update(b"\0");
        hasher.update(files[path].as_bytes());
        hasher.update(b"\0");
    }

    format!("{:x}", hasher.finalize())
}
