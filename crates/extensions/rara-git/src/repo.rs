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

use std::path::{Path, PathBuf};

use crate::error::GitError;

/// Wrapper around a `git2::Repository` with SSH key for remote operations.
pub struct GitRepo {
    inner:        git2::Repository,
    ssh_key_path: PathBuf,
}

impl GitRepo {
    /// Clone a repository via SSH.
    pub async fn clone_ssh(url: &str, target: &Path, ssh_key: &Path) -> Result<Self, GitError> {
        if !url.starts_with("git@") && !url.starts_with("ssh://") {
            return Err(GitError::InvalidUrl {
                url: url.to_owned(),
            });
        }

        let url = url.to_owned();
        let target = target.to_owned();
        let ssh_key_owned = ssh_key.to_owned();

        let (repo, key_path) = tokio::task::spawn_blocking(move || {
            let ssh_key_for_cb = ssh_key_owned.clone();
            let mut callbacks = git2::RemoteCallbacks::new();
            callbacks.credentials(move |_url, username, _allowed| {
                git2::Cred::ssh_key(username.unwrap_or("git"), None, &ssh_key_for_cb, None)
            });

            let mut fo = git2::FetchOptions::new();
            fo.remote_callbacks(callbacks);

            let repo = git2::build::RepoBuilder::new()
                .fetch_options(fo)
                .clone(&url, &target)
                .map_err(|e| GitError::CloneFailed {
                    message: e.to_string(),
                })?;

            Ok::<_, GitError>((repo, ssh_key_owned))
        })
        .await
        .map_err(|e| GitError::CloneFailed {
            message: format!("task join error: {e}"),
        })??;

        Ok(Self {
            inner:        repo,
            ssh_key_path: key_path,
        })
    }

    /// Open an existing local repository.
    pub fn open(path: &Path, ssh_key: &Path) -> Result<Self, GitError> {
        let repo = git2::Repository::open(path).map_err(|_e| GitError::RepoNotFound {
            path: path.display().to_string(),
        })?;
        Ok(Self {
            inner:        repo,
            ssh_key_path: ssh_key.to_owned(),
        })
    }

    /// Create a worktree on a new branch from HEAD.
    /// Worktree is placed at `{repo}/.worktrees/{name}/`.
    pub fn create_worktree(&self, name: &str, branch: &str) -> Result<PathBuf, GitError> {
        let repo_path = self.inner.workdir().ok_or_else(|| GitError::Worktree {
            message: "bare repository has no workdir".into(),
        })?;
        let wt_path = repo_path.join(".worktrees").join(name);

        let head = self.inner.head().map_err(|e| GitError::Worktree {
            message: format!("failed to get HEAD: {e}"),
        })?;
        let commit = head.peel_to_commit().map_err(|e| GitError::Worktree {
            message: format!("HEAD is not a commit: {e}"),
        })?;

        let branch_ref = match self.inner.branch(branch, &commit, false) {
            Ok(b) => b,
            Err(e) if e.code() == git2::ErrorCode::Exists => self
                .inner
                .find_branch(branch, git2::BranchType::Local)
                .map_err(|e| GitError::Worktree {
                    message: format!("failed to find branch: {e}"),
                })?,
            Err(e) => {
                return Err(GitError::Worktree {
                    message: format!("failed to create branch: {e}"),
                });
            }
        };

        let reference = branch_ref.into_reference();
        let mut opts = git2::WorktreeAddOptions::new();
        opts.reference(Some(&reference));

        std::fs::create_dir_all(wt_path.parent().unwrap_or(&wt_path)).map_err(|e| {
            GitError::Worktree {
                message: format!("failed to create worktree parent dir: {e}"),
            }
        })?;

        self.inner
            .worktree(name, &wt_path, Some(&opts))
            .map_err(|e| GitError::Worktree {
                message: format!("failed to add worktree: {e}"),
            })?;

        Ok(wt_path)
    }

    /// Remove a worktree and delete its directory.
    pub fn remove_worktree(&self, name: &str) -> Result<(), GitError> {
        let repo_path = self.inner.workdir().ok_or_else(|| GitError::Worktree {
            message: "bare repository has no workdir".into(),
        })?;
        let wt_path = repo_path.join(".worktrees").join(name);

        if wt_path.exists() {
            std::fs::remove_dir_all(&wt_path).map_err(|e| GitError::Worktree {
                message: format!("failed to remove worktree dir: {e}"),
            })?;
        }

        Ok(())
    }

    /// Stage all changes and commit. Returns `false` if nothing to commit.
    pub fn stage_and_commit(&self, message: &str, author: &str) -> Result<bool, GitError> {
        let mut index = self.inner.index().map_err(|e| GitError::Commit {
            message: format!("failed to get index: {e}"),
        })?;

        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .map_err(|e| GitError::Commit {
                message: format!("failed to stage: {e}"),
            })?;
        index.write().map_err(|e| GitError::Commit {
            message: format!("failed to write index: {e}"),
        })?;

        let tree_oid = index.write_tree().map_err(|e| GitError::Commit {
            message: format!("failed to write tree: {e}"),
        })?;
        let tree = self
            .inner
            .find_tree(tree_oid)
            .map_err(|e| GitError::Commit {
                message: format!("failed to find tree: {e}"),
            })?;

        // Check if tree differs from HEAD
        if let Ok(head) = self.inner.head() {
            if let Ok(head_commit) = head.peel_to_commit() {
                if head_commit.tree().map(|t| t.id()) == Ok(tree_oid) {
                    return Ok(false);
                }
            }
        }

        let sig = git2::Signature::now(author, &format!("{author}@rara")).map_err(|e| {
            GitError::Commit {
                message: format!("failed to create signature: {e}"),
            }
        })?;

        let parent = self.inner.head().ok().and_then(|h| h.peel_to_commit().ok());
        let parents: Vec<&git2::Commit<'_>> = parent.iter().collect();

        self.inner
            .commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)
            .map_err(|e| GitError::Commit {
                message: format!("failed to commit: {e}"),
            })?;

        Ok(true)
    }

    /// Push a branch to the remote `origin`.
    pub async fn push(&self, branch: &str) -> Result<(), GitError> {
        let repo_path = self
            .inner
            .workdir()
            .ok_or_else(|| GitError::Push {
                message: "bare repository".into(),
            })?
            .to_owned();
        let ssh_key = self.ssh_key_path.clone();
        let branch = branch.to_owned();

        tokio::task::spawn_blocking(move || {
            let repo = git2::Repository::open(&repo_path).map_err(|e| GitError::Push {
                message: format!("failed to open repo: {e}"),
            })?;
            let mut remote = repo.find_remote("origin").map_err(|e| GitError::Push {
                message: format!("no remote 'origin': {e}"),
            })?;

            let mut callbacks = git2::RemoteCallbacks::new();
            callbacks.credentials(move |_url, username, _allowed| {
                git2::Cred::ssh_key(username.unwrap_or("git"), None, &ssh_key, None)
            });

            let mut push_opts = git2::PushOptions::new();
            push_opts.remote_callbacks(callbacks);

            let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
            remote
                .push(&[&refspec], Some(&mut push_opts))
                .map_err(|e| GitError::Push {
                    message: e.to_string(),
                })
        })
        .await
        .map_err(|e| GitError::Push {
            message: format!("task join error: {e}"),
        })?
    }

    /// Fetch from origin and reset to match remote HEAD.
    pub async fn sync(&self) -> Result<(), GitError> {
        let repo_path = self
            .inner
            .workdir()
            .ok_or_else(|| GitError::Sync {
                message: "bare repository".into(),
            })?
            .to_owned();
        let ssh_key = self.ssh_key_path.clone();

        tokio::task::spawn_blocking(move || {
            let repo = git2::Repository::open(&repo_path).map_err(|e| GitError::Sync {
                message: format!("failed to open repo: {e}"),
            })?;
            let mut remote = repo.find_remote("origin").map_err(|e| GitError::Sync {
                message: format!("no remote 'origin': {e}"),
            })?;

            let mut callbacks = git2::RemoteCallbacks::new();
            callbacks.credentials(move |_url, username, _allowed| {
                git2::Cred::ssh_key(username.unwrap_or("git"), None, &ssh_key, None)
            });

            let mut fo = git2::FetchOptions::new();
            fo.remote_callbacks(callbacks);

            remote
                .fetch(&["refs/heads/*:refs/remotes/origin/*"], Some(&mut fo), None)
                .map_err(|e| GitError::Sync {
                    message: format!("fetch failed: {e}"),
                })?;

            let fetch_head = repo
                .find_reference("FETCH_HEAD")
                .map_err(|e| GitError::Sync {
                    message: format!("no FETCH_HEAD: {e}"),
                })?;
            let commit = fetch_head.peel_to_commit().map_err(|e| GitError::Sync {
                message: format!("FETCH_HEAD not a commit: {e}"),
            })?;

            repo.reset(commit.as_object(), git2::ResetType::Hard, None)
                .map_err(|e| GitError::Sync {
                    message: format!("reset failed: {e}"),
                })
        })
        .await
        .map_err(|e| GitError::Sync {
            message: format!("task join error: {e}"),
        })?
    }

    /// Get the path to the repository working directory.
    pub fn workdir(&self) -> Option<&Path> { self.inner.workdir() }
}
