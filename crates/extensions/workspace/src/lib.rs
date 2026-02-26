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

//! Git workspace pool management.
//!
//! [`WorkspaceManager`] handles cloning repos and creating git worktrees for
//! isolated coding task execution.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use tokio::process::Command;
use tracing::info;

/// Manages a pool of git workspaces under a configurable base directory.
///
/// Each repository is cloned once; subsequent tasks reuse the clone and create
/// isolated worktrees.
#[derive(Clone)]
pub struct WorkspaceManager {
    base_dir: PathBuf,
}

impl WorkspaceManager {
    pub fn new(base_dir: PathBuf) -> Self { Self { base_dir } }

    /// Sanitize a repo URL into a directory name.
    /// e.g. `"https://github.com/crrow/job"` → `"github.com-crrow-job"`
    fn repo_dir_name(repo_url: &str) -> String {
        repo_url
            .trim_end_matches('/')
            .trim_end_matches(".git")
            .replace("https://", "")
            .replace("http://", "")
            .replace('/', "-")
    }

    /// Get or create a workspace for the given repo.
    ///
    /// If the repo has been cloned before, fetches the latest changes and
    /// resets to `main`. Otherwise, clones the repo from scratch.
    pub async fn ensure_repo(&self, repo_url: &str) -> Result<PathBuf> {
        let dir_name = Self::repo_dir_name(repo_url);
        let repo_path = self.base_dir.join(&dir_name);

        if repo_path.exists() {
            info!(repo = %repo_url, path = %repo_path.display(), "reusing existing workspace");
            run_git(&repo_path, &["fetch", "origin"]).await?;
            run_git(&repo_path, &["checkout", "main"]).await?;
            run_git(&repo_path, &["pull", "--ff-only"]).await?;
        } else {
            info!(repo = %repo_url, path = %repo_path.display(), "cloning new workspace");
            tokio::fs::create_dir_all(&self.base_dir).await?;
            let out = Command::new("git")
                .args(["clone", repo_url, &dir_name])
                .current_dir(&self.base_dir)
                .output()
                .await
                .context("git clone failed")?;
            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr);
                bail!("git clone failed: {stderr}");
            }
        }
        Ok(repo_path)
    }

    /// Create a worktree branch for a task.
    ///
    /// Returns the path to the new worktree directory.
    pub async fn create_worktree(&self, repo_path: &Path, branch: &str) -> Result<PathBuf> {
        let worktree_path = repo_path.join(".worktrees").join(branch);
        let out = Command::new("git")
            .args(["worktree", "add"])
            .arg(&worktree_path)
            .args(["-b", branch, "main"])
            .current_dir(repo_path)
            .output()
            .await
            .context("git worktree add failed")?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            bail!("git worktree add failed: {stderr}");
        }
        info!(branch, path = %worktree_path.display(), "created worktree");
        Ok(worktree_path)
    }

    /// Cleanup a worktree after task completion.
    pub async fn cleanup_worktree(&self, repo_path: &Path, branch: &str) -> Result<()> {
        let worktree_path = repo_path.join(".worktrees").join(branch);
        let _ = Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(&worktree_path)
            .current_dir(repo_path)
            .output()
            .await;
        let _ = Command::new("git")
            .args(["branch", "-D", branch])
            .current_dir(repo_path)
            .output()
            .await;
        Ok(())
    }
}

/// Run a git command in the given directory and bail on failure.
async fn run_git(dir: &Path, args: &[&str]) -> Result<()> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .await
        .with_context(|| format!("git {} failed", args.join(" ")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        bail!("git {} failed: {stderr}", args.join(" "));
    }
    Ok(())
}
