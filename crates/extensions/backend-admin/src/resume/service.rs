use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

use super::repository::ResumeRepository;
use super::types::{ResumeError, ResumeProject, SetupResumeProjectRequest};

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
    pub fn new(repo: Arc<R>) -> Self {
        Self { repo }
    }

    /// Set up a new resume project: validate URL, clone repo, persist config.
    pub async fn setup(&self, req: SetupResumeProjectRequest) -> Result<ResumeProject, ResumeError> {
        // Check if already exists
        if self.repo.get().await?.is_some() {
            return Err(ResumeError::AlreadyExists);
        }

        // Validate SSH URL
        if !req.git_url.starts_with("git@") && !req.git_url.starts_with("ssh://") {
            return Err(ResumeError::InvalidGitUrl { url: req.git_url });
        }

        let id = Uuid::new_v4();
        let local_path = rara_paths::data_dir().join("resume").join(id.to_string());

        // Get SSH key
        let ssh_dir = rara_paths::data_dir().join("ssh");
        let keypair = rara_git::get_or_create_keypair(&ssh_dir)
            .map_err(|e| ResumeError::GitFailed {
                message: e.to_string(),
            })?;

        // Clone the repo
        rara_git::GitRepo::clone_ssh(&req.git_url, &local_path, &keypair.private_key_path)
            .await
            .map_err(|e| ResumeError::GitFailed {
                message: e.to_string(),
            })?;

        let local_path_str = local_path.display().to_string();
        self.repo
            .create(id, &req.name, &req.git_url, &local_path_str)
            .await?;

        // Update synced_at
        self.repo.update_synced_at(id).await?;

        // Re-fetch to get updated timestamp
        self.repo.get_by_id(id).await?.ok_or(ResumeError::NotFound)
    }

    /// Get the current resume project configuration.
    pub async fn get(&self) -> Result<Option<ResumeProject>, ResumeError> {
        self.repo.get().await
    }

    /// Sync (fetch + reset) the resume project from remote.
    pub async fn sync(&self) -> Result<ResumeProject, ResumeError> {
        let project = self.repo.get().await?.ok_or(ResumeError::NotFound)?;

        let ssh_dir = rara_paths::data_dir().join("ssh");
        let keypair = rara_git::get_or_create_keypair(&ssh_dir)
            .map_err(|e| ResumeError::GitFailed {
                message: e.to_string(),
            })?;

        // Run git open + sync inside spawn_blocking because
        // git2::Repository is !Send and cannot live across await points.
        let local_path: PathBuf = project.local_path.clone().into();
        let private_key = keypair.private_key_path.clone();
        tokio::task::spawn_blocking(move || sync_git_repo(&local_path, &private_key))
            .await
            .map_err(|e| ResumeError::GitFailed {
                message: format!("task join error: {e}"),
            })??;

        self.repo.update_synced_at(project.id).await?;
        self.repo
            .get_by_id(project.id)
            .await?
            .ok_or(ResumeError::NotFound)
    }

    /// Update the resume project name.
    pub async fn update_name(&self, name: &str) -> Result<ResumeProject, ResumeError> {
        let project = self.repo.get().await?.ok_or(ResumeError::NotFound)?;
        self.repo.update_name(project.id, name).await
    }

    /// Delete the resume project and remove local files.
    pub async fn delete(&self) -> Result<(), ResumeError> {
        let project = self.repo.get().await?.ok_or(ResumeError::NotFound)?;

        // Remove local clone
        let local_path = std::path::Path::new(&project.local_path);
        if local_path.exists() {
            std::fs::remove_dir_all(local_path).map_err(|e| ResumeError::GitFailed {
                message: format!("failed to remove local clone: {e}"),
            })?;
        }

        self.repo.delete(project.id).await
    }
}

/// Synchronous git fetch + reset, suitable for use inside `spawn_blocking`.
fn sync_git_repo(
    local_path: &std::path::Path,
    ssh_key: &std::path::Path,
) -> Result<(), ResumeError> {
    let repo = git2::Repository::open(local_path).map_err(|e| ResumeError::GitFailed {
        message: format!("failed to open repo: {e}"),
    })?;
    let mut remote = repo
        .find_remote("origin")
        .map_err(|e| ResumeError::GitFailed {
            message: format!("no remote 'origin': {e}"),
        })?;

    let ssh_key_owned = ssh_key.to_owned();
    let mut callbacks = git2::RemoteCallbacks::new();
    callbacks.credentials(move |_url, username, _allowed| {
        git2::Cred::ssh_key(username.unwrap_or("git"), None, &ssh_key_owned, None)
    });

    let mut fo = git2::FetchOptions::new();
    fo.remote_callbacks(callbacks);

    remote
        .fetch(
            &["refs/heads/*:refs/remotes/origin/*"],
            Some(&mut fo),
            None,
        )
        .map_err(|e| ResumeError::GitFailed {
            message: format!("fetch failed: {e}"),
        })?;

    let fetch_head = repo
        .find_reference("FETCH_HEAD")
        .map_err(|e| ResumeError::GitFailed {
            message: format!("no FETCH_HEAD: {e}"),
        })?;
    let commit = fetch_head
        .peel_to_commit()
        .map_err(|e| ResumeError::GitFailed {
            message: format!("FETCH_HEAD not a commit: {e}"),
        })?;

    repo.reset(commit.as_object(), git2::ResetType::Hard, None)
        .map_err(|e| ResumeError::GitFailed {
            message: format!("reset failed: {e}"),
        })?;

    Ok(())
}
