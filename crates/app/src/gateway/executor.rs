// Copyright 2025 Rararulab
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

//! [`UpdateExecutor`] — staging worktree build, binary activation & rollback.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use snafu::{ResultExt, Snafu};
use tokio::process::Command;
use tracing::{info, warn};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Snafu)]
pub enum ExecutorError {
    #[snafu(display("Failed to detect repo root: {source}"))]
    RepoDetect { source: std::io::Error },

    #[snafu(display("git rev-parse returned non-UTF-8 output"))]
    RepoDetectUtf8,

    #[snafu(display("git worktree add failed: {reason}"))]
    WorktreeAdd { reason: String },

    #[snafu(display("git worktree remove failed: {reason}"))]
    WorktreeRemove { reason: String },

    #[snafu(display("cargo build failed: {reason}"))]
    Build { reason: String },

    #[snafu(display("Build timed out after {timeout_secs}s"))]
    BuildTimeout { timeout_secs: u64 },

    #[snafu(display("Failed to resolve current executable path: {source}"))]
    CurrentExe { source: std::io::Error },

    #[snafu(display("Binary activation failed: {source}"))]
    Activation { source: std::io::Error },

    #[snafu(display("Failed to set executable permissions: {source}"))]
    Permissions { source: std::io::Error },

    #[snafu(display("Rollback failed: {source}"))]
    Rollback { source: std::io::Error },

    #[snafu(display("Cleanup failed: {source}"))]
    Cleanup { source: std::io::Error },
}

// ---------------------------------------------------------------------------
// UpdateResult
// ---------------------------------------------------------------------------

/// Outcome of an [`UpdateExecutor::execute_update`] call.
#[derive(Debug)]
pub enum UpdateResult {
    /// Build succeeded and the new binary is now active.
    Success { new_rev: String },
    /// `cargo build` failed (staging worktree has been cleaned up).
    BuildFailed { reason: String },
    /// Binary swap failed; `rolled_back` indicates whether the backup was
    /// successfully restored.
    ActivationFailed {
        reason:      String,
        rolled_back: bool,
    },
}

// ---------------------------------------------------------------------------
// UpdateExecutor
// ---------------------------------------------------------------------------

/// Builds a new binary in a staging git worktree and atomically swaps it
/// into the current executable path.
pub struct UpdateExecutor {
    repo_dir:    PathBuf,
    staging_dir: PathBuf,
    backup_path: PathBuf,
    exe_path:    PathBuf,
}

/// Build timeout: 10 minutes.
const BUILD_TIMEOUT: Duration = Duration::from_secs(600);

impl UpdateExecutor {
    /// Create a new executor.
    ///
    /// `repo_dir` is detected automatically via `git rev-parse --show-toplevel`
    /// on the directory containing the current executable.
    pub async fn new() -> Result<Self, ExecutorError> {
        let exe_path = std::env::current_exe().context(CurrentExeSnafu)?;
        let backup_path = exe_path.with_extension("bak");

        let repo_dir = detect_repo_root(&exe_path).await?;

        // Clean up any stale staging worktrees left by a previous crash.
        cleanup_stale_worktrees(&repo_dir).await;

        // staging_dir is set per-update in execute_update; use a placeholder.
        Ok(Self {
            repo_dir,
            staging_dir: PathBuf::new(),
            backup_path,
            exe_path,
        })
    }

    /// Execute a full update cycle: stage → build → activate.
    ///
    /// On build failure the staging worktree is cleaned up and
    /// [`UpdateResult::BuildFailed`] is returned (not an `Err`).
    /// On activation failure a rollback is attempted automatically.
    pub async fn execute_update(
        &mut self,
        target_rev: &str,
    ) -> Result<UpdateResult, ExecutorError> {
        let short_rev = &target_rev[..std::cmp::min(8, target_rev.len())];
        self.staging_dir = rara_paths::staging_dir().join(short_rev);

        // -- 1. Prepare staging worktree ------------------------------------
        info!(staging = %self.staging_dir.display(), "Preparing staging worktree");
        self.prepare_worktree(target_rev).await?;

        // -- 2. Build -------------------------------------------------------
        info!("Building release binary in staging worktree");
        match self.build().await {
            Ok(()) => {}
            Err(e) => {
                warn!(error = %e, "Build failed — cleaning up staging worktree");
                let _ = self.remove_worktree().await;
                return Ok(UpdateResult::BuildFailed {
                    reason: e.to_string(),
                });
            }
        }

        // -- 3. Activate ----------------------------------------------------
        info!("Activating new binary");
        match self.activate().await {
            Ok(()) => {
                info!(rev = short_rev, "Update activated successfully");
                Ok(UpdateResult::Success {
                    new_rev: target_rev.to_owned(),
                })
            }
            Err(e) => {
                warn!(error = %e, "Activation failed — attempting rollback");
                let rolled_back = self.rollback().await.is_ok();
                Ok(UpdateResult::ActivationFailed {
                    reason: e.to_string(),
                    rolled_back,
                })
            }
        }
    }

    /// Rollback: restore the `.bak` backup over the current exe path.
    pub async fn rollback(&self) -> Result<(), ExecutorError> {
        if self.backup_path.exists() {
            tokio::fs::rename(&self.backup_path, &self.exe_path)
                .await
                .context(RollbackSnafu)?;
            info!("Rolled back to previous binary");
        }
        let _ = self.remove_worktree().await;
        Ok(())
    }

    /// Clean up staging artifacts (backup file + worktree).
    pub async fn cleanup(&self) -> Result<(), ExecutorError> {
        // Remove .bak file if it exists.
        if self.backup_path.exists() {
            tokio::fs::remove_file(&self.backup_path)
                .await
                .context(CleanupSnafu)?;
        }
        self.remove_worktree().await?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Create (or recreate) the staging git worktree at `self.staging_dir`.
    async fn prepare_worktree(&self, target_rev: &str) -> Result<(), ExecutorError> {
        // If the staging dir already exists, remove the old worktree first.
        if self.staging_dir.exists() {
            let _ = self.remove_worktree().await;
        }

        let output = Command::new("git")
            .args([
                "worktree",
                "add",
                &self.staging_dir.to_string_lossy(),
                target_rev,
            ])
            .current_dir(&self.repo_dir)
            .output()
            .await
            .context(RepoDetectSnafu)?;

        if !output.status.success() {
            return Err(ExecutorError::WorktreeAdd {
                reason: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }

        Ok(())
    }

    /// Run `cargo build --release -p rara-cli` inside the staging worktree.
    async fn build(&self) -> Result<(), ExecutorError> {
        let mut child = Command::new("cargo")
            .args(["build", "--release", "-p", "rara-cli"])
            .env("CARGO_TARGET_DIR", self.repo_dir.join("target"))
            .env("RUSTC_WRAPPER", "sccache")
            .current_dir(&self.staging_dir)
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .map_err(|e| ExecutorError::Build {
                reason: e.to_string(),
            })?;

        let result = tokio::time::timeout(BUILD_TIMEOUT, child.wait()).await;

        match result {
            Ok(Ok(status)) if status.success() => Ok(()),
            Ok(Ok(status)) => Err(ExecutorError::Build {
                reason: format!("cargo build exited with {status}"),
            }),
            Ok(Err(e)) => Err(ExecutorError::Build {
                reason: e.to_string(),
            }),
            Err(_) => {
                // Timed out — kill the build process.
                let _ = child.kill().await;
                Err(ExecutorError::BuildTimeout {
                    timeout_secs: BUILD_TIMEOUT.as_secs(),
                })
            }
        }
    }

    /// Swap the current binary with the freshly-built one.
    ///
    /// 1. Rename current exe → `.bak`
    /// 2. Copy staging binary → current exe path
    /// 3. Set executable permissions (Unix)
    async fn activate(&self) -> Result<(), ExecutorError> {
        let new_binary = self.repo_dir.join("target/release/rara");

        if !new_binary.exists() {
            return Err(ExecutorError::Activation {
                source: std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("built binary not found at {}", new_binary.display()),
                ),
            });
        }

        // Backup current exe.
        tokio::fs::rename(&self.exe_path, &self.backup_path)
            .await
            .context(ActivationSnafu)?;

        // Copy new binary into place (copy, not rename, because staging may be
        // on a different filesystem / mount).
        tokio::fs::copy(&new_binary, &self.exe_path)
            .await
            .context(ActivationSnafu)?;

        // Set executable bit on Unix.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o755);
            tokio::fs::set_permissions(&self.exe_path, perms)
                .await
                .context(PermissionsSnafu)?;
        }

        // Advance local HEAD so the detector sees current_rev == upstream_rev
        // and stops triggering repeated builds.
        self.advance_local_head().await;

        Ok(())
    }

    /// Advance the local branch to match `origin/main` so the detector
    /// sees HEAD == upstream and stops re-triggering builds.
    ///
    /// Best-effort — a failure here is non-fatal (the binary is already
    /// activated). The next successful fetch cycle will just trigger one
    /// more redundant build in the worst case.
    async fn advance_local_head(&self) {
        let output = Command::new("git")
            .args(["merge", "--ff-only", "origin/main"])
            .current_dir(&self.repo_dir)
            .output()
            .await;

        match output {
            Ok(o) if o.status.success() => {
                info!("Advanced local HEAD to origin/main");
            }
            Ok(o) => {
                warn!(
                    stderr = %String::from_utf8_lossy(&o.stderr),
                    "git merge --ff-only origin/main failed (non-fatal)"
                );
            }
            Err(e) => {
                warn!(error = %e, "Failed to run git merge --ff-only (non-fatal)");
            }
        }
    }

    /// Remove the staging git worktree (best-effort).
    async fn remove_worktree(&self) -> Result<(), ExecutorError> {
        let output = Command::new("git")
            .args([
                "worktree",
                "remove",
                "--force",
                &self.staging_dir.to_string_lossy(),
            ])
            .current_dir(&self.repo_dir)
            .output()
            .await
            .context(RepoDetectSnafu)?;

        if !output.status.success() {
            return Err(ExecutorError::WorktreeRemove {
                reason: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }

        Ok(())
    }
}

/// Clean up stale staging worktrees that may have been left behind by a
/// previous crash, timeout, or SIGKILL.
///
/// This is best-effort: errors are logged but never propagated.
async fn cleanup_stale_worktrees(repo_dir: &Path) {
    // 1. Prune git's worktree bookkeeping for any already-deleted directories.
    match Command::new("git")
        .args(["worktree", "prune"])
        .current_dir(repo_dir)
        .output()
        .await
    {
        Ok(o) if o.status.success() => {
            info!("Pruned stale git worktree references");
        }
        Ok(o) => {
            warn!(
                stderr = %String::from_utf8_lossy(&o.stderr),
                "git worktree prune failed (non-fatal)"
            );
        }
        Err(e) => {
            warn!(error = %e, "Failed to run git worktree prune (non-fatal)");
        }
    }

    // 2. Remove leftover directories inside the staging root.
    let staging_root = rara_paths::staging_dir();
    let mut entries = match tokio::fs::read_dir(staging_root).await {
        Ok(entries) => entries,
        Err(e) => {
            // Directory may simply not exist yet — that's fine.
            if e.kind() != std::io::ErrorKind::NotFound {
                warn!(error = %e, "Failed to read staging directory (non-fatal)");
            }
            return;
        }
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.is_dir() {
            info!(path = %path.display(), "Removing stale staging directory");
            if let Err(e) = tokio::fs::remove_dir_all(&path).await {
                warn!(
                    path = %path.display(),
                    error = %e,
                    "Failed to remove stale staging directory (non-fatal)"
                );
            }
        }
    }
}

/// Detect the git repository root from the directory containing `exe_path`.
async fn detect_repo_root(exe_path: &std::path::Path) -> Result<PathBuf, ExecutorError> {
    let exe_dir = exe_path.parent().unwrap_or(exe_path);

    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(exe_dir)
        .output()
        .await
        .context(RepoDetectSnafu)?;

    if !output.status.success() {
        return Err(ExecutorError::RepoDetect {
            source: std::io::Error::other(
                format!(
                    "git rev-parse failed in {}: {}",
                    exe_dir.display(),
                    String::from_utf8_lossy(&output.stderr),
                ),
            ),
        });
    }

    let root = std::str::from_utf8(&output.stdout)
        .map_err(|_| ExecutorError::RepoDetectUtf8)?
        .trim()
        .to_owned();

    Ok(PathBuf::from(root))
}
