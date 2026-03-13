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

use std::{
    fs,
    path::{Path, PathBuf},
};

use snafu::{Location, ensure};
use tracing::{info, warn};

use crate::{
    config::RepoConfig,
    error::{Result, SymphonyError},
};

#[derive(Debug, Clone)]
pub struct WorkspaceInfo {
    pub path:        PathBuf,
    pub branch:      String,
    pub created_now: bool,
}

#[derive(Debug, Clone)]
pub struct WorkspaceManager;

impl WorkspaceManager {
    fn ensure_repo_checkout(&self, repo: &RepoConfig) -> Result<(git2::Repository, PathBuf)> {
        ensure!(
            repo.repo_path.is_some(),
            crate::error::WorkspaceSnafu {
                message: format!("repo {} is missing repo_path", repo.name),
            }
        );
        let repo_path = repo.repo_path.clone().expect("checked repo_path above");

        if repo_path.exists() {
            let checkout =
                git2::Repository::open(&repo_path).map_err(|source| SymphonyError::Git {
                    source,
                    location: Location::new(file!(), line!(), column!()),
                })?;
            return Ok((checkout, repo_path));
        }

        if let Some(parent) = repo_path.parent() {
            fs::create_dir_all(parent).map_err(|source| SymphonyError::Io {
                source,
                location: Location::new(file!(), line!(), column!()),
            })?;
        }

        info!(
            repo = %repo.name,
            url = %repo.url,
            path = %repo_path.display(),
            "cloning missing repository checkout for symphony"
        );

        let checkout = git2::Repository::clone(&repo.url, &repo_path).map_err(|source| {
            SymphonyError::Git {
                source,
                location: Location::new(file!(), line!(), column!()),
            }
        })?;

        Ok((checkout, repo_path))
    }

    fn invalid_existing_worktree(path: &Path) -> bool {
        path.exists() && git2::Repository::open(path).is_err()
    }

    /// Ensure the issue branch is checked out in a dedicated worktree under the
    /// configured workspace root. Existing worktrees are reused.
    pub fn ensure_worktree(
        &self,
        repo: &RepoConfig,
        issue_number: u64,
        issue_title: &str,
    ) -> Result<WorkspaceInfo> {
        ensure!(
            repo.repo_path.is_some(),
            crate::error::WorkspaceSnafu {
                message: format!("repo {} is missing repo_path", repo.name),
            }
        );
        ensure!(
            repo.effective_workspace_root().is_some(),
            crate::error::WorkspaceSnafu {
                message: format!("repo {} is missing workspace_root", repo.name),
            }
        );
        let workspace_root = repo
            .effective_workspace_root()
            .expect("checked workspace_root above");
        let branch = branch_name(issue_number, issue_title);
        let path = workspace_root.join(&branch);
        let (checkout, _) = self.ensure_repo_checkout(repo)?;

        if path.exists() {
            if Self::invalid_existing_worktree(&path) {
                warn!(
                    path = %path.display(),
                    branch = %branch,
                    "removing invalid existing symphony worktree before recreation"
                );
                fs::remove_dir_all(&path).map_err(|source| SymphonyError::WorkspaceIo {
                    message: format!(
                        "failed to remove invalid worktree {} for repo {} branch {}",
                        path.display(),
                        repo.name,
                        branch
                    ),
                    source,
                    location: Location::new(file!(), line!(), column!()),
                })?;
                if let Ok(wt) = checkout.find_worktree(&branch) {
                    let _ = wt.prune(Some(
                        git2::WorktreePruneOptions::new().valid(false).locked(false),
                    ));
                }
            } else {
                return Ok(WorkspaceInfo {
                    path,
                    branch,
                    created_now: false,
                });
            }
        }

        fs::create_dir_all(&workspace_root).map_err(|source| SymphonyError::Io {
            source,
            location: Location::new(file!(), line!(), column!()),
        })?;
        let head_ref = checkout.head().map_err(|source| SymphonyError::Git {
            source,
            location: Location::new(file!(), line!(), column!()),
        })?;
        let head = head_ref
            .peel_to_commit()
            .map_err(|source| SymphonyError::Git {
                source,
                location: Location::new(file!(), line!(), column!()),
            })?;

        let branch_ref = match checkout.branch(&branch, &head, false) {
            Ok(branch_ref) => branch_ref,
            Err(err) if err.code() == git2::ErrorCode::Exists => checkout
                .find_branch(&branch, git2::BranchType::Local)
                .map_err(|source| SymphonyError::Git {
                    source,
                    location: Location::new(file!(), line!(), column!()),
                })?,
            Err(err) => {
                return Err(SymphonyError::Git {
                    source:   err,
                    location: Location::new(file!(), line!(), column!()),
                });
            }
        };

        let reference = branch_ref.into_reference();
        let mut options = git2::WorktreeAddOptions::new();
        options.reference(Some(&reference));
        checkout
            .worktree(&branch, &path, Some(&options))
            .map_err(|source| SymphonyError::Git {
                source,
                location: Location::new(file!(), line!(), column!()),
            })?;

        Ok(WorkspaceInfo {
            path,
            branch,
            created_now: true,
        })
    }

    /// Remove the issue worktree and prune the matching git worktree/branch.
    pub fn cleanup_worktree(&self, repo: &RepoConfig, workspace: &WorkspaceInfo) -> Result<()> {
        ensure!(
            repo.repo_path.is_some(),
            crate::error::WorkspaceSnafu {
                message: format!("repo {} is missing repo_path", repo.name),
            }
        );
        let (repo, _) = self.ensure_repo_checkout(repo)?;

        if workspace.path.exists() {
            fs::remove_dir_all(&workspace.path).map_err(|source| SymphonyError::Io {
                source,
                location: Location::new(file!(), line!(), column!()),
            })?;
        }

        if let Ok(wt) = repo.find_worktree(&workspace.branch) {
            let _ = wt.prune(Some(
                git2::WorktreePruneOptions::new().valid(false).locked(false),
            ));
        }

        if let Ok(mut branch) = repo.find_branch(&workspace.branch, git2::BranchType::Local) {
            branch.delete().map_err(|source| SymphonyError::Git {
                source,
                location: Location::new(file!(), line!(), column!()),
            })?;
        }

        Ok(())
    }
}

/// Generate a stable per-issue branch name from the issue number and title.
fn branch_name(issue_number: u64, issue_title: &str) -> String {
    let slug = issue_title
        .to_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|piece| !piece.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug = if slug.is_empty() {
        "task".to_owned()
    } else {
        slug
    };
    format!("issue-{issue_number}-{slug}")
}

/// Resolve the repo-specific workflow file path relative to the worktree root.
pub fn workflow_file(repo: &RepoConfig, default_workflow_file: &str) -> PathBuf {
    Path::new(
        repo.workflow_file
            .as_deref()
            .unwrap_or(default_workflow_file),
    )
    .to_path_buf()
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn creates_and_cleans_up_worktree() {
        let repo_dir = TempDir::new().unwrap();
        let workspace_root = TempDir::new().unwrap();
        let repo = git2::Repository::init(repo_dir.path()).unwrap();
        let mut index = repo.index().unwrap();
        let tree_oid = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let sig = git2::Signature::now("test", "test@example.com").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .unwrap();

        let repo = RepoConfig::builder()
            .name("rararulab/rara".to_owned())
            .url("https://github.com/rararulab/rara".to_owned())
            .repo_path(repo_dir.path().to_path_buf())
            .workspace_root(workspace_root.path().to_path_buf())
            .active_labels(vec!["symphony:ready".to_owned()])
            .build();

        let manager = WorkspaceManager;
        let workspace = manager.ensure_worktree(&repo, 42, "Fix startup").unwrap();
        assert!(workspace.path.exists());
        assert_eq!(workspace.branch, "issue-42-fix-startup");

        manager.cleanup_worktree(&repo, &workspace).unwrap();
        assert!(!workspace.path.exists());
    }

    #[test]
    fn clones_missing_repo_before_creating_worktree() {
        let source_dir = TempDir::new().unwrap();
        let workspace_root = TempDir::new().unwrap();
        let source_repo = git2::Repository::init(source_dir.path()).unwrap();
        let mut index = source_repo.index().unwrap();
        let tree_oid = index.write_tree().unwrap();
        let tree = source_repo.find_tree(tree_oid).unwrap();
        let sig = git2::Signature::now("test", "test@example.com").unwrap();
        source_repo
            .commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .unwrap();

        let checkout_root = TempDir::new().unwrap();
        let repo_path = checkout_root.path().join("clones/repo");
        let repo = RepoConfig::builder()
            .name("crrowbot/rara-notes".to_owned())
            .url(source_dir.path().display().to_string())
            .repo_path(repo_path.clone())
            .workspace_root(workspace_root.path().to_path_buf())
            .active_labels(vec!["symphony:ready".to_owned()])
            .build();

        let manager = WorkspaceManager;
        let workspace = manager.ensure_worktree(&repo, 11, "Dynamic repo").unwrap();

        assert!(repo_path.exists());
        assert!(workspace.path.exists());
    }

    #[test]
    fn replaces_invalid_existing_worktree_directory() {
        let repo_dir = TempDir::new().unwrap();
        let repo = git2::Repository::init(repo_dir.path()).unwrap();
        std::fs::write(repo_dir.path().join("ralph.core.yml"), "agent: {}\n").unwrap();
        let mut index = repo.index().unwrap();
        index
            .add_path(Path::new("ralph.core.yml"))
            .expect("should stage core config");
        let tree_oid = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let sig = git2::Signature::now("test", "test@example.com").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .unwrap();

        let workspace_root = TempDir::new().unwrap();
        let broken_path = workspace_root.path().join("issue-11-dynamic-repo");
        std::fs::create_dir_all(&broken_path).unwrap();
        std::fs::write(
            broken_path.join(".git"),
            "gitdir: /tmp/nonexistent-symphony-worktree\n",
        )
        .unwrap();

        let repo = RepoConfig::builder()
            .name("crrowbot/rara-notes".to_owned())
            .url(repo_dir.path().display().to_string())
            .repo_path(repo_dir.path().to_path_buf())
            .workspace_root(workspace_root.path().to_path_buf())
            .active_labels(vec!["symphony:ready".to_owned()])
            .build();

        let manager = WorkspaceManager;
        let workspace = manager.ensure_worktree(&repo, 11, "Dynamic repo").unwrap();

        assert!(workspace.created_now);
        assert!(workspace.path.join("ralph.core.yml").exists());
        assert!(git2::Repository::open(&workspace.path).is_ok());
    }
}
