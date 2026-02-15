//! Application-level service for Typst project management and compilation.
//!
//! Files are read from and written to the local filesystem. The database stores
//! only project metadata and render history.

use std::{collections::HashMap, path::Path, sync::Arc};

use opendal::Operator;
use sha2::{Digest, Sha256};
use tracing::instrument;
use uuid::Uuid;

use crate::{
    compiler,
    error::{TypstError, map_storage_err},
    fs::{self, FileEntry},
    git::GitImporter,
    repository::TypstRepository,
    types::{ImportGitRequest, RegisterProjectRequest, RenderResult, TypstProject},
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

    /// Register a local project directory.
    ///
    /// Validates that the path exists, is a directory, and contains at least
    /// one `.typ` file.
    #[instrument(skip(self))]
    pub async fn register_project(
        &self,
        req: RegisterProjectRequest,
    ) -> Result<TypstProject, TypstError> {
        if req.name.trim().is_empty() {
            return Err(TypstError::InvalidRequest {
                message: "project name must not be empty".to_owned(),
            });
        }

        let local_path = Path::new(&req.local_path);

        if !local_path.exists() {
            return Err(TypstError::DirectoryNotFound {
                path: req.local_path.clone(),
            });
        }
        if !local_path.is_dir() {
            return Err(TypstError::NotADirectory {
                path: req.local_path.clone(),
            });
        }

        // Check that there is at least one .typ file.
        let typ_files = fs::collect_typ_files(local_path)?;
        if typ_files.is_empty() {
            return Err(TypstError::NoTypstFiles {
                path: req.local_path.clone(),
            });
        }

        // Auto-detect main file.
        let main_file = req.main_file.unwrap_or_else(|| {
            if typ_files.contains_key("main.typ") {
                "main.typ".to_owned()
            } else {
                typ_files.keys().next().cloned().unwrap_or_else(|| "main.typ".to_owned())
            }
        });

        self.repo
            .create_project(&req.name, &req.local_path, &main_file, None)
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

    /// Delete a project (database record and S3 renders only; local files are NOT deleted).
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

    // -- File operations (local filesystem) --

    /// List the file tree for a project by scanning its local directory.
    pub fn list_files(&self, project: &TypstProject) -> Result<Vec<FileEntry>, TypstError> {
        fs::scan_directory(Path::new(&project.local_path))
    }

    /// Read a file's content from disk.
    pub fn read_file(&self, project: &TypstProject, path: &str) -> Result<String, TypstError> {
        fs::read_file(Path::new(&project.local_path), path)
    }

    /// Write content to a file on disk.
    pub fn write_file(
        &self,
        project: &TypstProject,
        path: &str,
        content: &str,
    ) -> Result<(), TypstError> {
        fs::write_file(Path::new(&project.local_path), path, content)
    }

    // -- Git integration --

    /// Import a Typst project from a Git repository.
    ///
    /// Clones the repo into `target_dir`, then registers it as a local project.
    #[instrument(skip(self))]
    pub async fn import_from_git(
        &self,
        request: ImportGitRequest,
    ) -> Result<TypstProject, TypstError> {
        let target = Path::new(&request.target_dir);

        // Clone into target directory.
        let importer = GitImporter;
        importer.clone_to(&request.url, target).await?;

        // Verify there are .typ files.
        let typ_files = fs::collect_typ_files(target)?;
        if typ_files.is_empty() {
            return Err(TypstError::NoTypstFiles {
                path: request.target_dir.clone(),
            });
        }

        // Auto-detect main file.
        let main_file = if typ_files.contains_key("main.typ") {
            "main.typ".to_owned()
        } else {
            typ_files.keys().next().cloned().unwrap_or_else(|| "main.typ".to_owned())
        };

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
            .create_project(&name, &request.target_dir, &main_file, Some(&request.url))
            .await?;

        // Update sync timestamp.
        let project = self.repo.update_git_synced(project.id).await?;

        tracing::info!(
            project_id = %project.id,
            file_count = typ_files.len(),
            git_url = %request.url,
            "imported project from git"
        );

        Ok(project)
    }

    /// Sync a Git-backed project by pulling the latest changes.
    #[instrument(skip(self))]
    pub async fn sync_git(&self, project_id: Uuid) -> Result<TypstProject, TypstError> {
        let project = self
            .repo
            .get_project(project_id)
            .await?
            .ok_or(TypstError::ProjectNotFound { id: project_id })?;

        let git_url = project.git_url.as_deref().ok_or(TypstError::NotGitProject)?;

        // Re-clone into the same directory (the importer handles this).
        let importer = GitImporter;
        importer.clone_to(git_url, Path::new(&project.local_path)).await?;

        // Update sync timestamp.
        let project = self.repo.update_git_synced(project_id).await?;

        tracing::info!(
            project_id = %project_id,
            "synced project from git"
        );

        Ok(project)
    }

    // -- Compilation --

    /// Compile a project to PDF.
    ///
    /// Reads all `.typ` files from the local filesystem and compiles them.
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

        // Gather all .typ files from disk.
        let file_map = fs::collect_typ_files(Path::new(&project.local_path))?;
        if file_map.is_empty() {
            return Err(TypstError::InvalidRequest {
                message: "project has no .typ files".to_owned(),
            });
        }

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
