//! Git repository import and sync operations.

use std::path::Path;
use std::time::Duration;

use tracing::instrument;

use crate::error::TypstError;

/// Clone timeout.
const CLONE_TIMEOUT: Duration = Duration::from_secs(60);

/// Encapsulates Git clone operations.
pub struct GitImporter;

impl GitImporter {
    /// Clone a Git repository from `url` into the given `target_dir`.
    ///
    /// If the target directory already exists, it is removed first (full replace).
    ///
    /// # Security constraints
    /// - Only `https://` URLs are accepted.
    /// - Clone times out after 60 seconds.
    #[instrument(skip(self))]
    pub async fn clone_to(&self, url: &str, target_dir: &Path) -> Result<(), TypstError> {
        validate_url(url)?;

        let url = url.to_owned();
        let target = target_dir.to_owned();

        let result = tokio::time::timeout(
            CLONE_TIMEOUT,
            tokio::task::spawn_blocking(move || clone_into(&url, &target)),
        )
        .await;

        match result {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => Err(TypstError::GitCloneFailed {
                message: format!("blocking task panicked: {e}"),
            }),
            Err(_elapsed) => Err(TypstError::GitCloneFailed {
                message: "git clone timed out after 60 seconds".to_owned(),
            }),
        }
    }
}

/// Validate that the URL starts with `https://`.
fn validate_url(url: &str) -> Result<(), TypstError> {
    if !url.starts_with("https://") {
        return Err(TypstError::InvalidGitUrl {
            url: url.to_owned(),
        });
    }
    Ok(())
}

/// Clone a repository into the specified directory.
fn clone_into(url: &str, target: &Path) -> Result<(), TypstError> {
    // If the directory exists, remove it for a clean clone.
    if target.exists() {
        std::fs::remove_dir_all(target).map_err(|e| TypstError::GitCloneFailed {
            message: format!("failed to clean target directory: {e}"),
        })?;
    }

    // Create parent directories if needed.
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| TypstError::GitCloneFailed {
            message: format!("failed to create parent directories: {e}"),
        })?;
    }

    // Clone (shallow, depth=1) for speed.
    let mut fetch_opts = git2::FetchOptions::new();
    fetch_opts.depth(1);

    let mut builder = git2::build::RepoBuilder::new();
    builder.fetch_options(fetch_opts);

    builder
        .clone(url, target)
        .map_err(|e| TypstError::GitCloneFailed {
            message: e.to_string(),
        })?;

    Ok(())
}
