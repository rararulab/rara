use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tracing::{debug, info, warn};

use crate::config::{HooksConfig, RepoConfig};
use crate::error::{Result, SymphonyError};
use crate::event::WorkspaceInfo;

/// Internal per-repo workspace configuration.
#[derive(Debug, Clone)]
struct RepoWorkspaceConfig {
    repo_path: PathBuf,
    workspace_root: PathBuf,
    hooks: HooksConfig,
}

/// Manages git worktree lifecycles for symphony-tracked repositories.
#[derive(Debug)]
pub struct WorkspaceManager {
    repos: HashMap<String, RepoWorkspaceConfig>,
}

impl WorkspaceManager {
    /// Build a workspace manager from the configured repositories.
    pub fn new(repo_configs: &[RepoConfig]) -> Self {
        let repos = repo_configs
            .iter()
            .map(|rc| {
                (
                    rc.name.clone(),
                    RepoWorkspaceConfig {
                        repo_path: rc.repo_path.clone(),
                        workspace_root: rc.workspace_root.clone(),
                        hooks: rc.hooks.clone(),
                    },
                )
            })
            .collect();
        Self { repos }
    }

    /// Create (or reuse) a worktree for the given issue.
    pub fn ensure_worktree(
        &self,
        repo_name: &str,
        issue_number: u64,
        issue_title: &str,
    ) -> Result<WorkspaceInfo> {
        let cfg = self.repo_config(repo_name)?;
        let branch = worktree_branch_name(issue_number, issue_title);
        let wt_path = cfg.workspace_root.join(&branch);

        // If the worktree directory already exists, reuse it.
        if wt_path.exists() {
            info!(repo = repo_name, branch = %branch, "reusing existing worktree");
            return Ok(WorkspaceInfo {
                path: wt_path,
                branch,
                created_now: false,
            });
        }

        // Open the main repository.
        let repo = git2::Repository::open(&cfg.repo_path).map_err(|e| SymphonyError::Git {
            source: e,
        })?;

        // Resolve HEAD commit and create the branch.
        let head_commit = repo
            .head()
            .and_then(|r| r.peel_to_commit())
            .map_err(|e| SymphonyError::Git { source: e })?;

        repo.branch(&branch, &head_commit, false)
            .map_err(|e| SymphonyError::Git { source: e })?;

        debug!(repo = repo_name, branch = %branch, "created branch");

        // Create the worktree via git2.
        let mut wt_opts = git2::WorktreeAddOptions::new();
        let reference = repo
            .find_branch(&branch, git2::BranchType::Local)
            .map_err(|e| SymphonyError::Git { source: e })?
            .into_reference();
        wt_opts.reference(Some(&reference));

        repo.worktree(&branch, &wt_path, Some(&wt_opts))
            .map_err(|e| SymphonyError::Git { source: e })?;

        info!(repo = repo_name, branch = %branch, path = %wt_path.display(), "created worktree");

        Ok(WorkspaceInfo {
            path: wt_path,
            branch,
            created_now: true,
        })
    }

    /// Remove a worktree, prune its git reference, and delete the branch.
    pub fn cleanup_worktree(
        &self,
        repo_name: &str,
        workspace: &WorkspaceInfo,
    ) -> Result<()> {
        let cfg = self.repo_config(repo_name)?;

        // Remove the worktree directory from disk.
        if workspace.path.exists() {
            std::fs::remove_dir_all(&workspace.path).map_err(|e| SymphonyError::Io {
                source: e,
            })?;
            debug!(path = %workspace.path.display(), "removed worktree directory");
        }

        // Open the repo and prune stale worktrees.
        let repo = git2::Repository::open(&cfg.repo_path).map_err(|e| SymphonyError::Git {
            source: e,
        })?;

        // Prune the worktree reference if it exists.
        if let Ok(wt) = repo.find_worktree(&workspace.branch) {
            if wt.validate().is_err() {
                wt.prune(Some(
                    git2::WorktreePruneOptions::new().valid(false).locked(false),
                ))
                .map_err(|e| SymphonyError::Git { source: e })?;
                debug!(branch = %workspace.branch, "pruned worktree reference");
            }
        }

        // Delete the branch.
        if let Ok(mut branch) = repo.find_branch(&workspace.branch, git2::BranchType::Local) {
            branch
                .delete()
                .map_err(|e| SymphonyError::Git { source: e })?;
            info!(branch = %workspace.branch, "deleted branch");
        } else {
            warn!(branch = %workspace.branch, "branch not found during cleanup");
        }

        Ok(())
    }

    /// Return the hooks configuration for a repository, if it exists.
    pub fn hooks_for(&self, repo_name: &str) -> Option<&HooksConfig> {
        self.repos.get(repo_name).map(|c| &c.hooks)
    }

    fn repo_config(&self, repo_name: &str) -> Result<&RepoWorkspaceConfig> {
        self.repos.get(repo_name).ok_or_else(|| {
            SymphonyError::Workspace {
                message: format!("unknown repository: {repo_name}"),
            }
        })
    }
}

/// Run a hook script in the given directory.
pub async fn run_hook(hook_script: &str, cwd: &Path) -> Result<()> {
    let output = tokio::process::Command::new("sh")
        .arg("-lc")
        .arg(hook_script)
        .current_dir(cwd)
        .output()
        .await
        .map_err(|e| SymphonyError::Io { source: e })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(SymphonyError::Hook {
            hook: hook_script.to_owned(),
            message: format!(
                "exit code {:?}: {}",
                output.status.code(),
                stderr.trim()
            ),
        });
    }

    Ok(())
}

/// Build a sanitised branch name from an issue number and title.
fn worktree_branch_name(issue_number: u64, title: &str) -> String {
    let sanitized: String = title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();

    // Collapse consecutive dashes and trim leading/trailing dashes.
    let collapsed = sanitized
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");

    // Truncate to 40 characters (at a dash boundary if possible).
    let truncated = if collapsed.len() > 40 {
        let slice = &collapsed[..40];
        // Try to cut at the last dash within the limit.
        match slice.rfind('-') {
            Some(pos) if pos > 0 => &slice[..pos],
            _ => slice,
        }
    } else {
        &collapsed
    };

    format!("issue-{issue_number}-{truncated}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RepoConfig;
    use tempfile::TempDir;

    #[test]
    fn branch_name_sanitization() {
        let name = worktree_branch_name(42, "Fix: the BUG! (urgent)");
        assert_eq!(name, "issue-42-fix-the-bug-urgent");
    }

    #[test]
    fn branch_name_truncation() {
        let long_title =
            "this is a very long issue title that should definitely be truncated somewhere";
        let name = worktree_branch_name(99, long_title);

        // The sanitised slug portion (after "issue-99-") must be <= 40 chars.
        let slug = name.strip_prefix("issue-99-").unwrap();
        assert!(
            slug.len() <= 40,
            "slug length {} exceeds 40: {slug}",
            slug.len()
        );
        // Should not end with a dash.
        assert!(!slug.ends_with('-'), "slug ends with dash: {slug}");
    }

    #[test]
    fn worktree_create_and_cleanup() {
        // Set up a temporary bare-ish repo with an initial commit.
        let repo_dir = TempDir::new().unwrap();
        let repo = git2::Repository::init(repo_dir.path()).unwrap();

        // Create an initial commit so HEAD is valid.
        {
            let mut index = repo.index().unwrap();
            let tree_oid = index.write_tree().unwrap();
            let tree = repo.find_tree(tree_oid).unwrap();
            let sig = git2::Signature::now("test", "test@test.com").unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
                .unwrap();
        }

        let ws_root = TempDir::new().unwrap();

        let repo_cfg = RepoConfig::builder()
            .name("test-repo".to_owned())
            .url("https://example.com/repo.git".to_owned())
            .repo_path(repo_dir.path().to_path_buf())
            .workspace_root(ws_root.path().to_path_buf())
            .active_labels(vec!["symphony:ready".to_owned()])
            .hooks(HooksConfig::default())
            .build();

        let mgr = WorkspaceManager::new(&[repo_cfg]);

        // Create a worktree.
        let ws = mgr
            .ensure_worktree("test-repo", 7, "my feature")
            .expect("should create worktree");
        assert!(ws.created_now, "first call should create");
        assert!(ws.path.exists(), "worktree directory should exist");
        assert_eq!(ws.branch, "issue-7-my-feature");

        // Reuse existing worktree.
        let ws2 = mgr
            .ensure_worktree("test-repo", 7, "my feature")
            .expect("should reuse worktree");
        assert!(!ws2.created_now, "second call should reuse");

        // Cleanup.
        mgr.cleanup_worktree("test-repo", &ws)
            .expect("should cleanup");
        assert!(!ws.path.exists(), "worktree directory should be gone");

        // Branch should be deleted.
        let repo_reopened = git2::Repository::open(repo_dir.path()).unwrap();
        assert!(
            repo_reopened
                .find_branch("issue-7-my-feature", git2::BranchType::Local)
                .is_err(),
            "branch should be deleted"
        );
    }
}
