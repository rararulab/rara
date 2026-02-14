//! Git repository import and sync operations.

use std::path::Path;
use std::time::Duration;

use tracing::instrument;

use crate::error::TypstError;

/// Maximum total size of scanned files (50 MB).
const MAX_REPO_SIZE: u64 = 50 * 1024 * 1024;

/// Clone timeout.
const CLONE_TIMEOUT: Duration = Duration::from_secs(60);

/// File extensions that are imported from Git repositories.
const ALLOWED_EXTENSIONS: &[&str] = &["typ", "bib", "csv", "yaml", "yml", "json", "toml"];

/// A file imported from a Git repository.
#[derive(Debug, Clone)]
pub struct ImportedFile {
    /// Relative path within the repository.
    pub path: String,
    /// Full text content of the file.
    pub content: String,
}

/// Encapsulates Git clone-and-scan operations.
pub struct GitImporter;

impl GitImporter {
    /// Clone a Git repository from `url`, scan for supported files, and return
    /// their contents.
    ///
    /// # Security constraints
    /// - Only `https://` URLs are accepted.
    /// - Clone times out after 60 seconds.
    /// - Total scanned file size must be under 50 MB.
    #[instrument(skip(self))]
    pub async fn import_from_url(&self, url: &str) -> Result<Vec<ImportedFile>, TypstError> {
        validate_url(url)?;

        let url = url.to_owned();
        // git2 is blocking, so run in a blocking task with a timeout.
        let result = tokio::time::timeout(
            CLONE_TIMEOUT,
            tokio::task::spawn_blocking(move || clone_and_scan(&url)),
        )
        .await;

        match result {
            Ok(Ok(files)) => files,
            Ok(Err(e)) => Err(TypstError::GitCloneFailed {
                message: format!("blocking task panicked: {e}"),
            }),
            Err(_elapsed) => Err(TypstError::GitCloneFailed {
                message: "git clone timed out after 60 seconds".to_owned(),
            }),
        }
    }

    /// Re-clone and return the latest files. Semantically identical to
    /// `import_from_url` but named for clarity at the call site.
    pub async fn sync(&self, url: &str) -> Result<Vec<ImportedFile>, TypstError> {
        self.import_from_url(url).await
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

/// Clone a repository into a temporary directory and scan for supported files.
fn clone_and_scan(url: &str) -> Result<Vec<ImportedFile>, TypstError> {
    let tmp_dir = tempfile::tempdir().map_err(|e| TypstError::GitCloneFailed {
        message: format!("failed to create temp dir: {e}"),
    })?;

    // Clone (shallow, depth=1) for speed.
    let mut fetch_opts = git2::FetchOptions::new();
    fetch_opts.depth(1);

    let mut builder = git2::build::RepoBuilder::new();
    builder.fetch_options(fetch_opts);

    builder
        .clone(url, tmp_dir.path())
        .map_err(|e| TypstError::GitCloneFailed {
            message: e.to_string(),
        })?;

    // Scan files.
    let files = scan_directory(tmp_dir.path())?;

    // tmp_dir is automatically cleaned up on drop.
    Ok(files)
}

/// Recursively scan a directory for supported files and read their contents.
fn scan_directory(root: &Path) -> Result<Vec<ImportedFile>, TypstError> {
    let mut files = Vec::new();
    let mut total_size: u64 = 0;
    scan_recursive(root, root, &mut files, &mut total_size)?;
    Ok(files)
}

fn scan_recursive(
    root: &Path,
    dir: &Path,
    files: &mut Vec<ImportedFile>,
    total_size: &mut u64,
) -> Result<(), TypstError> {
    let entries = std::fs::read_dir(dir).map_err(|e| TypstError::GitCloneFailed {
        message: format!("failed to read directory {}: {e}", dir.display()),
    })?;

    for entry in entries {
        let entry = entry.map_err(|e| TypstError::GitCloneFailed {
            message: format!("failed to read dir entry: {e}"),
        })?;

        let path = entry.path();

        // Skip hidden directories (e.g. `.git`).
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with('.') {
                continue;
            }
        }

        if path.is_dir() {
            scan_recursive(root, &path, files, total_size)?;
        } else if path.is_file() {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");

            if !ALLOWED_EXTENSIONS.contains(&ext) {
                continue;
            }

            let metadata = std::fs::metadata(&path).map_err(|e| TypstError::GitCloneFailed {
                message: format!("failed to stat file {}: {e}", path.display()),
            })?;

            *total_size += metadata.len();
            if *total_size > MAX_REPO_SIZE {
                return Err(TypstError::RepositoryTooLarge { size: *total_size });
            }

            let content = std::fs::read_to_string(&path).map_err(|e| TypstError::GitCloneFailed {
                message: format!("failed to read file {}: {e}", path.display()),
            })?;

            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();

            files.push(ImportedFile {
                path: relative,
                content,
            });
        }
    }

    Ok(())
}
