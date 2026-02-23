# Resume Refactor Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Refactor resume from document management to GitHub Typst project management, create reusable `rara-git` extension crate with SSH key management, remove Typst domain crate.

**Architecture:** Resume becomes a GitHub repo URL (Typst project) cloned locally. Git operations unified into `rara-git` extension. SSH Ed25519 key pairs generated and managed by rara for GitHub auth. Compilation delegated to agents via bash tools. Typst domain crate and frontend pages removed entirely.

**Tech Stack:** git2, ssh-key (pure Rust Ed25519), snafu, axum, sqlx, React 19, TanStack Query

---

### Task 1: Create `rara-git` extension crate — scaffold + error types

**Files:**
- Create: `crates/extensions/rara-git/Cargo.toml`
- Create: `crates/extensions/rara-git/src/lib.rs`
- Create: `crates/extensions/rara-git/src/error.rs`
- Modify: `Cargo.toml` (workspace root, add member + dependency)

**Step 1: Create Cargo.toml**

```toml
[package]
name = "rara-git"
version.workspace = true
edition.workspace = true
license.workspace = true
description = "Git operations library — SSH key management, clone, worktree, commit, push"

[lints]
workspace = true

[dependencies]
git2 = { workspace = true }
rand_core = { version = "0.6", features = ["getrandom"] }
rara-paths = { workspace = true }
snafu.workspace = true
ssh-key = { version = "0.6", features = ["ed25519", "rand_core"] }
tokio = { workspace = true }
tracing.workspace = true
```

**Step 2: Create error.rs**

```rust
use snafu::Snafu;

#[derive(Debug, Snafu)]
pub enum GitError {
    #[snafu(display("invalid git URL: {url}"))]
    InvalidUrl { url: String },

    #[snafu(display("clone failed: {message}"))]
    CloneFailed { message: String },

    #[snafu(display("repository not found at {path}"))]
    RepoNotFound { path: String },

    #[snafu(display("worktree error: {message}"))]
    Worktree { message: String },

    #[snafu(display("commit error: {message}"))]
    Commit { message: String },

    #[snafu(display("push error: {message}"))]
    Push { message: String },

    #[snafu(display("sync error: {message}"))]
    Sync { message: String },

    #[snafu(display("SSH key error: {message}"))]
    SshKey { message: String },

    #[snafu(display("IO error: {source}"))]
    Io { source: std::io::Error },
}
```

**Step 3: Create lib.rs**

```rust
pub mod error;
pub mod repo;
pub mod ssh;

pub use error::GitError;
pub use repo::GitRepo;
pub use ssh::{get_or_create_keypair, get_public_key, SshKeyPair};
```

**Step 4: Add to workspace root `Cargo.toml`**

In `[workspace] members` array, add:
```
"crates/extensions/rara-git",
```

In `[workspace.dependencies]` section, add:
```
rara-git = { path = "crates/extensions/rara-git" }
```

Also add `ssh-key` if not already in workspace deps:
```
ssh-key = { version = "0.6", features = ["ed25519", "rand_core"] }
```

**Step 5: Verify scaffold compiles**

Run: `cargo check -p rara-git`
Expected: Compile errors about missing `repo` and `ssh` modules (that's fine, they come in next tasks)

**Step 6: Commit**

```bash
git add crates/extensions/rara-git/ Cargo.toml Cargo.lock
git commit -m "feat(git): scaffold rara-git extension crate with error types"
```

---

### Task 2: Implement SSH key management (`ssh.rs`)

**Files:**
- Create: `crates/extensions/rara-git/src/ssh.rs`

**Step 1: Write unit test**

At bottom of `ssh.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn generates_keypair_when_none_exists() {
        let tmp = TempDir::new().unwrap();
        let pair = get_or_create_keypair(tmp.path()).unwrap();
        assert!(!pair.public_key.is_empty());
        assert!(pair.public_key.starts_with("ssh-ed25519 "));
        // Files created
        assert!(tmp.path().join("id_ed25519").exists());
        assert!(tmp.path().join("id_ed25519.pub").exists());
    }

    #[test]
    fn returns_existing_keypair() {
        let tmp = TempDir::new().unwrap();
        let pair1 = get_or_create_keypair(tmp.path()).unwrap();
        let pair2 = get_or_create_keypair(tmp.path()).unwrap();
        assert_eq!(pair1.public_key, pair2.public_key);
    }

    #[test]
    fn get_public_key_creates_if_needed() {
        let tmp = TempDir::new().unwrap();
        let pk = get_public_key(tmp.path()).unwrap();
        assert!(pk.starts_with("ssh-ed25519 "));
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p rara-git ssh::tests`
Expected: FAIL — function not defined

**Step 3: Implement ssh.rs**

```rust
use std::path::{Path, PathBuf};

use crate::error::GitError;

/// An SSH Ed25519 key pair.
#[derive(Debug, Clone)]
pub struct SshKeyPair {
    /// OpenSSH-formatted public key string (e.g. "ssh-ed25519 AAAA...")
    pub public_key:      String,
    /// Path to private key file
    pub private_key_path: PathBuf,
}

/// Get or create an Ed25519 SSH key pair in the given directory.
///
/// Keys are stored as `{ssh_dir}/id_ed25519` and `{ssh_dir}/id_ed25519.pub`.
/// If keys already exist, they are loaded and returned. Otherwise, a new pair
/// is generated.
pub fn get_or_create_keypair(ssh_dir: &Path) -> Result<SshKeyPair, GitError> {
    let private_path = ssh_dir.join("id_ed25519");
    let public_path = ssh_dir.join("id_ed25519.pub");

    if private_path.exists() && public_path.exists() {
        let public_key = std::fs::read_to_string(&public_path).map_err(|e| {
            GitError::SshKey { message: format!("failed to read public key: {e}") }
        })?;
        return Ok(SshKeyPair {
            public_key: public_key.trim().to_owned(),
            private_key_path: private_path,
        });
    }

    // Generate new Ed25519 key pair
    std::fs::create_dir_all(ssh_dir).map_err(|e| {
        GitError::SshKey { message: format!("failed to create SSH directory: {e}") }
    })?;

    let private_key = ssh_key::PrivateKey::random(
        &mut rand_core::OsRng,
        ssh_key::Algorithm::Ed25519,
    ).map_err(|e| {
        GitError::SshKey { message: format!("failed to generate key: {e}") }
    })?;

    // Write private key (OpenSSH format, no passphrase)
    let private_pem = private_key.to_openssh(ssh_key::LineEnding::LF).map_err(|e| {
        GitError::SshKey { message: format!("failed to serialize private key: {e}") }
    })?;
    std::fs::write(&private_path, private_pem.as_bytes()).map_err(|e| {
        GitError::SshKey { message: format!("failed to write private key: {e}") }
    })?;

    // Set permissions to 600 on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&private_path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| GitError::SshKey { message: format!("failed to set key permissions: {e}") })?;
    }

    // Write public key
    let public_key = private_key.public_key().to_openssh().map_err(|e| {
        GitError::SshKey { message: format!("failed to serialize public key: {e}") }
    })?;
    std::fs::write(&public_path, public_key.as_bytes()).map_err(|e| {
        GitError::SshKey { message: format!("failed to write public key: {e}") }
    })?;

    Ok(SshKeyPair {
        public_key: public_key.trim().to_owned(),
        private_key_path: private_path,
    })
}

/// Get the public key string, generating a key pair if none exists.
pub fn get_public_key(ssh_dir: &Path) -> Result<String, GitError> {
    let pair = get_or_create_keypair(ssh_dir)?;
    Ok(pair.public_key)
}
```

**Step 4: Run tests**

Run: `cargo test -p rara-git ssh::tests`
Expected: All 3 tests PASS

**Step 5: Add `tempfile` dev-dependency to Cargo.toml**

```toml
[dev-dependencies]
tempfile.workspace = true
```

**Step 6: Commit**

```bash
git add crates/extensions/rara-git/
git commit -m "feat(git): implement SSH Ed25519 key pair generation"
```

---

### Task 3: Implement Git repo operations (`repo.rs`)

**Files:**
- Create: `crates/extensions/rara-git/src/repo.rs`

**Step 1: Write tests**

At bottom of `repo.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn init_repo(dir: &Path) -> git2::Repository {
        let repo = git2::Repository::init(dir).unwrap();
        // Create initial commit so HEAD exists
        let sig = git2::Signature::now("test", "test@test.com").unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[]).unwrap();
        repo
    }

    #[test]
    fn open_existing_repo() {
        let tmp = TempDir::new().unwrap();
        let ssh_tmp = TempDir::new().unwrap();
        let ssh_key = crate::ssh::get_or_create_keypair(ssh_tmp.path()).unwrap();
        init_repo(tmp.path());
        let repo = GitRepo::open(tmp.path(), &ssh_key.private_key_path);
        assert!(repo.is_ok());
    }

    #[test]
    fn create_and_remove_worktree() {
        let tmp = TempDir::new().unwrap();
        let ssh_tmp = TempDir::new().unwrap();
        let ssh_key = crate::ssh::get_or_create_keypair(ssh_tmp.path()).unwrap();
        init_repo(tmp.path());
        let repo = GitRepo::open(tmp.path(), &ssh_key.private_key_path).unwrap();

        let wt_path = repo.create_worktree("test-wt", "test-branch").unwrap();
        assert!(wt_path.exists());

        repo.remove_worktree("test-wt").unwrap();
        assert!(!wt_path.exists());
    }

    #[test]
    fn stage_and_commit_detects_changes() {
        let tmp = TempDir::new().unwrap();
        let ssh_tmp = TempDir::new().unwrap();
        let ssh_key = crate::ssh::get_or_create_keypair(ssh_tmp.path()).unwrap();
        init_repo(tmp.path());
        let repo = GitRepo::open(tmp.path(), &ssh_key.private_key_path).unwrap();

        // No changes → false
        let committed = repo.stage_and_commit("empty", "rara").unwrap();
        assert!(!committed);

        // Write a file → true
        std::fs::write(tmp.path().join("test.txt"), "hello").unwrap();
        let committed = repo.stage_and_commit("add test", "rara").unwrap();
        assert!(committed);
    }
}
```

**Step 2: Run tests to verify failure**

Run: `cargo test -p rara-git repo::tests`
Expected: FAIL — `GitRepo` not defined

**Step 3: Implement repo.rs**

```rust
use std::path::{Path, PathBuf};

use crate::error::GitError;

/// Wrapper around a `git2::Repository` with SSH key for remote operations.
pub struct GitRepo {
    inner: git2::Repository,
    ssh_key_path: PathBuf,
}

impl GitRepo {
    /// Clone a repository via SSH.
    pub async fn clone_ssh(url: &str, target: &Path, ssh_key: &Path) -> Result<Self, GitError> {
        // Validate SSH URL format
        if !url.starts_with("git@") && !url.starts_with("ssh://") {
            return Err(GitError::InvalidUrl { url: url.to_owned() });
        }

        let url = url.to_owned();
        let target = target.to_owned();
        let ssh_key = ssh_key.to_owned();

        let repo = tokio::task::spawn_blocking(move || {
            let mut callbacks = git2::RemoteCallbacks::new();
            callbacks.credentials(move |_url, username, _allowed| {
                git2::Cred::ssh_key(
                    username.unwrap_or("git"),
                    None,
                    &ssh_key,
                    None,
                )
            });

            let mut fo = git2::FetchOptions::new();
            fo.remote_callbacks(callbacks);

            git2::build::RepoBuilder::new()
                .fetch_options(fo)
                .clone(&url, &target)
        })
        .await
        .map_err(|e| GitError::CloneFailed { message: format!("task join error: {e}") })?
        .map_err(|e| GitError::CloneFailed { message: e.to_string() })?;

        Ok(Self {
            inner: repo,
            ssh_key_path: ssh_key.to_owned(), // Note: ssh_key was moved, use original
        })
    }

    /// Open an existing local repository.
    pub fn open(path: &Path, ssh_key: &Path) -> Result<Self, GitError> {
        let repo = git2::Repository::open(path).map_err(|e| {
            GitError::RepoNotFound { path: path.display().to_string() }
        })?;
        Ok(Self {
            inner: repo,
            ssh_key_path: ssh_key.to_owned(),
        })
    }

    /// Create a worktree on a new branch from HEAD.
    ///
    /// Worktree is placed at `{repo}/.worktrees/{name}/`.
    pub fn create_worktree(&self, name: &str, branch: &str) -> Result<PathBuf, GitError> {
        let repo_path = self.inner.workdir().ok_or_else(|| {
            GitError::Worktree { message: "bare repository has no workdir".into() }
        })?;
        let wt_path = repo_path.join(".worktrees").join(name);

        // Create branch from HEAD
        let head = self.inner.head().map_err(|e| {
            GitError::Worktree { message: format!("failed to get HEAD: {e}") }
        })?;
        let commit = head.peel_to_commit().map_err(|e| {
            GitError::Worktree { message: format!("HEAD is not a commit: {e}") }
        })?;

        let branch_ref = match self.inner.branch(branch, &commit, false) {
            Ok(b) => b,
            Err(e) if e.code() == git2::ErrorCode::Exists => {
                self.inner.find_branch(branch, git2::BranchType::Local).map_err(|e| {
                    GitError::Worktree { message: format!("failed to find branch: {e}") }
                })?
            }
            Err(e) => return Err(GitError::Worktree { message: format!("failed to create branch: {e}") }),
        };

        let reference = branch_ref.into_reference();
        let mut opts = git2::WorktreeAddOptions::new();
        opts.reference(Some(&reference));

        self.inner.worktree(name, &wt_path, Some(&opts)).map_err(|e| {
            GitError::Worktree { message: format!("failed to add worktree: {e}") }
        })?;

        Ok(wt_path)
    }

    /// Remove a worktree and delete its directory.
    pub fn remove_worktree(&self, name: &str) -> Result<(), GitError> {
        let repo_path = self.inner.workdir().ok_or_else(|| {
            GitError::Worktree { message: "bare repository has no workdir".into() }
        })?;
        let wt_path = repo_path.join(".worktrees").join(name);

        // Remove the worktree directory
        if wt_path.exists() {
            std::fs::remove_dir_all(&wt_path).map_err(|e| {
                GitError::Worktree { message: format!("failed to remove worktree dir: {e}") }
            })?;
        }

        // Prune worktree references
        let _ = self.inner.worktree(name, &wt_path, None); // ignore if already gone

        Ok(())
    }

    /// Stage all changes and commit. Returns `false` if nothing to commit.
    pub fn stage_and_commit(&self, message: &str, author: &str) -> Result<bool, GitError> {
        let mut index = self.inner.index().map_err(|e| {
            GitError::Commit { message: format!("failed to get index: {e}") }
        })?;

        index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None).map_err(|e| {
            GitError::Commit { message: format!("failed to stage: {e}") }
        })?;
        index.write().map_err(|e| {
            GitError::Commit { message: format!("failed to write index: {e}") }
        })?;

        let tree_oid = index.write_tree().map_err(|e| {
            GitError::Commit { message: format!("failed to write tree: {e}") }
        })?;
        let tree = self.inner.find_tree(tree_oid).map_err(|e| {
            GitError::Commit { message: format!("failed to find tree: {e}") }
        })?;

        // Check if tree differs from HEAD
        if let Ok(head) = self.inner.head() {
            if let Ok(head_commit) = head.peel_to_commit() {
                if head_commit.tree().map(|t| t.id()) == Ok(tree_oid) {
                    return Ok(false); // Nothing changed
                }
            }
        }

        let sig = git2::Signature::now(author, &format!("{author}@rara")).map_err(|e| {
            GitError::Commit { message: format!("failed to create signature: {e}") }
        })?;

        let parent = self.inner.head().ok().and_then(|h| h.peel_to_commit().ok());
        let parents: Vec<&git2::Commit<'_>> = parent.iter().collect();

        self.inner.commit(Some("HEAD"), &sig, &sig, message, &tree, &parents).map_err(|e| {
            GitError::Commit { message: format!("failed to commit: {e}") }
        })?;

        Ok(true)
    }

    /// Push a branch to the remote `origin`.
    pub async fn push(&self, branch: &str) -> Result<(), GitError> {
        let repo_path = self.inner.workdir().ok_or_else(|| {
            GitError::Push { message: "bare repository".into() }
        })?.to_owned();
        let ssh_key = self.ssh_key_path.clone();
        let branch = branch.to_owned();

        tokio::task::spawn_blocking(move || {
            let repo = git2::Repository::open(&repo_path).map_err(|e| {
                GitError::Push { message: format!("failed to open repo: {e}") }
            })?;

            let mut remote = repo.find_remote("origin").map_err(|e| {
                GitError::Push { message: format!("no remote 'origin': {e}") }
            })?;

            let mut callbacks = git2::RemoteCallbacks::new();
            callbacks.credentials(move |_url, username, _allowed| {
                git2::Cred::ssh_key(username.unwrap_or("git"), None, &ssh_key, None)
            });

            let mut push_opts = git2::PushOptions::new();
            push_opts.remote_callbacks(callbacks);

            let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
            remote.push(&[&refspec], Some(&mut push_opts)).map_err(|e| {
                GitError::Push { message: e.to_string() }
            })?;

            Ok(())
        })
        .await
        .map_err(|e| GitError::Push { message: format!("task join error: {e}") })?
    }

    /// Fetch from origin and reset to match remote HEAD.
    pub async fn sync(&self) -> Result<(), GitError> {
        let repo_path = self.inner.workdir().ok_or_else(|| {
            GitError::Sync { message: "bare repository".into() }
        })?.to_owned();
        let ssh_key = self.ssh_key_path.clone();

        tokio::task::spawn_blocking(move || {
            let repo = git2::Repository::open(&repo_path).map_err(|e| {
                GitError::Sync { message: format!("failed to open repo: {e}") }
            })?;

            let mut remote = repo.find_remote("origin").map_err(|e| {
                GitError::Sync { message: format!("no remote 'origin': {e}") }
            })?;

            let mut callbacks = git2::RemoteCallbacks::new();
            callbacks.credentials(move |_url, username, _allowed| {
                git2::Cred::ssh_key(username.unwrap_or("git"), None, &ssh_key, None)
            });

            let mut fo = git2::FetchOptions::new();
            fo.remote_callbacks(callbacks);

            remote.fetch(&["refs/heads/*:refs/remotes/origin/*"], Some(&mut fo), None)
                .map_err(|e| GitError::Sync { message: format!("fetch failed: {e}") })?;

            // Reset HEAD to origin/main (or whatever the default branch is)
            let fetch_head = repo.find_reference("FETCH_HEAD").map_err(|e| {
                GitError::Sync { message: format!("no FETCH_HEAD: {e}") }
            })?;
            let commit = fetch_head.peel_to_commit().map_err(|e| {
                GitError::Sync { message: format!("FETCH_HEAD not a commit: {e}") }
            })?;

            repo.reset(commit.as_object(), git2::ResetType::Hard, None).map_err(|e| {
                GitError::Sync { message: format!("reset failed: {e}") }
            })?;

            Ok(())
        })
        .await
        .map_err(|e| GitError::Sync { message: format!("task join error: {e}") })?
    }

    /// Get the path to the repository working directory.
    pub fn workdir(&self) -> Option<&Path> {
        self.inner.workdir()
    }
}
```

**Step 4: Run tests**

Run: `cargo test -p rara-git repo::tests`
Expected: All 3 tests PASS

**Step 5: Commit**

```bash
git add crates/extensions/rara-git/src/repo.rs
git commit -m "feat(git): implement GitRepo — clone, worktree, commit, push, sync"
```

---

### Task 4: Add SSH key API to settings routes

**Files:**
- Modify: `crates/domain/shared/src/settings/router.rs` (add SSH key endpoint)

**Step 1: Add SSH key endpoint**

Add to settings router a `GET /api/v1/settings/ssh-key` endpoint that:
1. Calls `rara_git::get_or_create_keypair()` with `{data_dir}/ssh/`
2. Returns `{ "public_key": "ssh-ed25519 AAAA..." }`

Add `rara-git` dependency to `rara-domain-shared` Cargo.toml.

Response type:
```rust
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct SshKeyResponse {
    pub public_key: String,
}
```

Handler:
```rust
async fn get_ssh_key() -> Result<Json<SshKeyResponse>, (StatusCode, String)> {
    let ssh_dir = rara_paths::data_dir().join("ssh");
    let public_key = rara_git::get_public_key(&ssh_dir)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(SshKeyResponse { public_key }))
}
```

Wire into existing `routes()` function.

**Step 2: Verify**

Run: `cargo check -p rara-domain-shared`

**Step 3: Commit**

```bash
git add crates/domain/shared/
git commit -m "feat(settings): add SSH public key API endpoint"
```

---

### Task 5: Database migration — drop old tables, create resume_project

**Files:**
- Create: `crates/domain/resume/migrations/YYYYMMDDHHMMSS_resume_project.up.sql`
- Create: `crates/domain/resume/migrations/YYYYMMDDHHMMSS_resume_project.down.sql`

**Step 1: Write up migration**

```sql
-- Drop old resume table and related objects
DROP TABLE IF EXISTS resume CASCADE;

-- Drop old typst tables
DROP TABLE IF EXISTS typst_render CASCADE;
DROP TABLE IF EXISTS typst_project CASCADE;

-- Create new resume_project table
CREATE TABLE resume_project (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name            TEXT NOT NULL,
    git_url         TEXT NOT NULL,
    local_path      TEXT NOT NULL,
    last_synced_at  TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TRIGGER set_updated_at BEFORE UPDATE ON resume_project
    FOR EACH ROW EXECUTE FUNCTION trigger_set_updated_at();
```

**Step 2: Write down migration**

```sql
DROP TABLE IF EXISTS resume_project CASCADE;
```

**Step 3: Commit**

```bash
git add crates/domain/resume/migrations/
git commit -m "feat(resume): add migration to replace resume table with resume_project"
```

---

### Task 6: Rewrite resume domain crate — types + error

**Files:**
- Rewrite: `crates/domain/resume/src/types.rs`
- Delete: `crates/domain/resume/src/hash.rs`
- Delete: `crates/domain/resume/src/version.rs`
- Modify: `crates/domain/resume/src/lib.rs`
- Modify: `crates/domain/resume/Cargo.toml`

**Step 1: Rewrite types.rs**

Replace entire file with:

```rust
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use snafu::Snafu;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Domain model
// ---------------------------------------------------------------------------

/// A resume project backed by a GitHub repository.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ResumeProject {
    pub id: Uuid,
    pub name: String,
    pub git_url: String,
    pub local_path: String,
    #[schema(value_type = Option<String>)]
    pub last_synced_at: Option<Timestamp>,
    #[schema(value_type = String)]
    pub created_at: Timestamp,
    #[schema(value_type = String)]
    pub updated_at: Timestamp,
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

/// Request to set up a new resume project.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SetupResumeProjectRequest {
    pub name: String,
    pub git_url: String,
}

/// Request to update a resume project.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateResumeProjectRequest {
    pub name: Option<String>,
}

// ---------------------------------------------------------------------------
// DB row
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ResumeProjectRow {
    pub id: Uuid,
    pub name: String,
    pub git_url: String,
    pub local_path: String,
    pub last_synced_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<ResumeProjectRow> for ResumeProject {
    fn from(row: ResumeProjectRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            git_url: row.git_url,
            local_path: row.local_path,
            last_synced_at: row.last_synced_at.map(|t| {
                rara_domain_shared::convert::chrono_to_timestamp(t)
            }),
            created_at: rara_domain_shared::convert::chrono_to_timestamp(row.created_at),
            updated_at: rara_domain_shared::convert::chrono_to_timestamp(row.updated_at),
        }
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Snafu)]
pub enum ResumeError {
    #[snafu(display("resume project not found"))]
    NotFound,

    #[snafu(display("resume project already exists"))]
    AlreadyExists,

    #[snafu(display("invalid git URL: {url}"))]
    InvalidGitUrl { url: String },

    #[snafu(display("git operation failed: {message}"))]
    GitFailed { message: String },

    #[snafu(display("repository error: {source}"))]
    Repository { source: sqlx::Error },
}

impl axum::response::IntoResponse for ResumeError {
    fn into_response(self) -> axum::response::Response {
        let (status, msg) = match &self {
            Self::NotFound => (axum::http::StatusCode::NOT_FOUND, self.to_string()),
            Self::AlreadyExists => (axum::http::StatusCode::CONFLICT, self.to_string()),
            Self::InvalidGitUrl { .. } => (axum::http::StatusCode::BAD_REQUEST, self.to_string()),
            Self::GitFailed { .. } => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            Self::Repository { .. } => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };
        (status, axum::Json(serde_json::json!({ "error": msg }))).into_response()
    }
}
```

**Step 2: Simplify Cargo.toml dependencies**

Remove dependencies no longer needed: `sha2`, `bytes`, `opendal`, `strum`, `strum_macros`, multipart feature from axum.
Add: `rara-git = { workspace = true }`, `rara-paths = { workspace = true }`.

**Step 3: Update lib.rs**

```rust
use std::sync::Arc;
use sqlx::PgPool;

pub mod pg_repository;
pub mod repository;
pub mod router;
pub mod service;
pub mod types;

pub type ResumeAppService = service::ResumeService<pg_repository::PgResumeRepository>;

#[must_use]
pub fn wire_resume_service(pool: PgPool) -> ResumeAppService {
    let repo = Arc::new(pg_repository::PgResumeRepository::new(pool));
    service::ResumeService::new(repo)
}
```

**Step 4: Delete old files**

```bash
rm crates/domain/resume/src/hash.rs crates/domain/resume/src/version.rs
```

**Step 5: Verify**

Run: `cargo check -p rara-domain-resume`
Expected: Errors about repository/service/routes referencing old types (fixed in next tasks)

**Step 6: Commit**

```bash
git add crates/domain/resume/
git commit -m "refactor(resume): rewrite types for GitHub repo model, remove version/hash"
```

---

### Task 7: Rewrite resume repository + pg_repository

**Files:**
- Rewrite: `crates/domain/resume/src/repository.rs`
- Rewrite: `crates/domain/resume/src/pg_repository.rs`

**Step 1: Rewrite repository.rs**

```rust
use uuid::Uuid;
use crate::types::{ResumeError, ResumeProject};

#[async_trait::async_trait]
pub trait ResumeRepository: Send + Sync {
    async fn create(&self, id: Uuid, name: &str, git_url: &str, local_path: &str) -> Result<ResumeProject, ResumeError>;
    async fn get(&self) -> Result<Option<ResumeProject>, ResumeError>;
    async fn get_by_id(&self, id: Uuid) -> Result<Option<ResumeProject>, ResumeError>;
    async fn update_name(&self, id: Uuid, name: &str) -> Result<ResumeProject, ResumeError>;
    async fn update_synced_at(&self, id: Uuid) -> Result<(), ResumeError>;
    async fn delete(&self, id: Uuid) -> Result<(), ResumeError>;
}
```

**Step 2: Rewrite pg_repository.rs**

```rust
use sqlx::PgPool;
use uuid::Uuid;

use crate::repository::ResumeRepository;
use crate::types::{ResumeError, ResumeProject, ResumeProjectRow};

pub struct PgResumeRepository {
    pool: PgPool,
}

impl PgResumeRepository {
    pub fn new(pool: PgPool) -> Self { Self { pool } }
}

#[async_trait::async_trait]
impl ResumeRepository for PgResumeRepository {
    async fn create(&self, id: Uuid, name: &str, git_url: &str, local_path: &str) -> Result<ResumeProject, ResumeError> {
        let row: ResumeProjectRow = sqlx::query_as(
            "INSERT INTO resume_project (id, name, git_url, local_path) VALUES ($1, $2, $3, $4) RETURNING *"
        )
        .bind(id)
        .bind(name)
        .bind(git_url)
        .bind(local_path)
        .fetch_one(&self.pool)
        .await
        .map_err(|source| ResumeError::Repository { source })?;

        Ok(row.into())
    }

    async fn get(&self) -> Result<Option<ResumeProject>, ResumeError> {
        let row: Option<ResumeProjectRow> = sqlx::query_as(
            "SELECT * FROM resume_project ORDER BY created_at ASC LIMIT 1"
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|source| ResumeError::Repository { source })?;

        Ok(row.map(Into::into))
    }

    async fn get_by_id(&self, id: Uuid) -> Result<Option<ResumeProject>, ResumeError> {
        let row: Option<ResumeProjectRow> = sqlx::query_as(
            "SELECT * FROM resume_project WHERE id = $1"
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|source| ResumeError::Repository { source })?;

        Ok(row.map(Into::into))
    }

    async fn update_name(&self, id: Uuid, name: &str) -> Result<ResumeProject, ResumeError> {
        let row: ResumeProjectRow = sqlx::query_as(
            "UPDATE resume_project SET name = $2 WHERE id = $1 RETURNING *"
        )
        .bind(id)
        .bind(name)
        .fetch_one(&self.pool)
        .await
        .map_err(|source| ResumeError::Repository { source })?;

        Ok(row.into())
    }

    async fn update_synced_at(&self, id: Uuid) -> Result<(), ResumeError> {
        sqlx::query("UPDATE resume_project SET last_synced_at = now() WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|source| ResumeError::Repository { source })?;
        Ok(())
    }

    async fn delete(&self, id: Uuid) -> Result<(), ResumeError> {
        sqlx::query("DELETE FROM resume_project WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|source| ResumeError::Repository { source })?;
        Ok(())
    }
}
```

**Step 3: Verify**

Run: `cargo check -p rara-domain-resume`

**Step 4: Commit**

```bash
git add crates/domain/resume/src/repository.rs crates/domain/resume/src/pg_repository.rs
git commit -m "refactor(resume): rewrite repository for resume_project model"
```

---

### Task 8: Rewrite resume service

**Files:**
- Rewrite: `crates/domain/resume/src/service.rs`

**Step 1: Implement new service**

```rust
use std::sync::Arc;
use uuid::Uuid;

use crate::repository::ResumeRepository;
use crate::types::{ResumeError, ResumeProject, SetupResumeProjectRequest};

#[derive(Clone)]
pub struct ResumeService<R: ResumeRepository> {
    repo: Arc<R>,
}

impl<R: ResumeRepository> ResumeService<R> {
    pub fn new(repo: Arc<R>) -> Self { Self { repo } }

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
            .map_err(|e| ResumeError::GitFailed { message: e.to_string() })?;

        // Clone the repo
        rara_git::GitRepo::clone_ssh(&req.git_url, &local_path, &keypair.private_key_path)
            .await
            .map_err(|e| ResumeError::GitFailed { message: e.to_string() })?;

        let local_path_str = local_path.display().to_string();
        let project = self.repo.create(id, &req.name, &req.git_url, &local_path_str).await?;

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
            .map_err(|e| ResumeError::GitFailed { message: e.to_string() })?;

        let local_path = std::path::Path::new(&project.local_path);
        let git_repo = rara_git::GitRepo::open(local_path, &keypair.private_key_path)
            .map_err(|e| ResumeError::GitFailed { message: e.to_string() })?;

        git_repo.sync().await
            .map_err(|e| ResumeError::GitFailed { message: e.to_string() })?;

        self.repo.update_synced_at(project.id).await?;
        self.repo.get_by_id(project.id).await?.ok_or(ResumeError::NotFound)
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
            std::fs::remove_dir_all(local_path).map_err(|e| {
                ResumeError::GitFailed { message: format!("failed to remove local clone: {e}") }
            })?;
        }

        self.repo.delete(project.id).await
    }
}
```

**Step 2: Verify**

Run: `cargo check -p rara-domain-resume`

**Step 3: Commit**

```bash
git add crates/domain/resume/src/service.rs
git commit -m "refactor(resume): rewrite service for GitHub repo workflow"
```

---

### Task 9: Rewrite resume routes

**Files:**
- Rewrite: `crates/domain/resume/src/router.rs`

**Step 1: Implement new routes**

```rust
use axum::{
    extract::State,
    http::StatusCode,
    Json,
};
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::repository::ResumeRepository;
use crate::service::ResumeService;
use crate::types::{ResumeError, ResumeProject, SetupResumeProjectRequest, UpdateResumeProjectRequest};

#[derive(Clone)]
struct RouteState<R: ResumeRepository> {
    service: ResumeService<R>,
}

pub fn routes<R: ResumeRepository + 'static>(service: ResumeService<R>) -> OpenApiRouter {
    let state = RouteState { service };
    OpenApiRouter::new()
        .routes(routes!(setup_project::<R>))
        .routes(routes!(get_project::<R>))
        .routes(routes!(update_project::<R>))
        .routes(routes!(delete_project::<R>))
        .routes(routes!(sync_project::<R>))
        .with_state(state)
}

#[utoipa::path(
    post,
    path = "/api/v1/resume-project",
    tag = "resume",
    request_body = SetupResumeProjectRequest,
    responses(
        (status = 201, description = "Project created", body = ResumeProject),
    )
)]
async fn setup_project<R: ResumeRepository + 'static>(
    State(state): State<RouteState<R>>,
    Json(req): Json<SetupResumeProjectRequest>,
) -> Result<(StatusCode, Json<ResumeProject>), ResumeError> {
    let project = state.service.setup(req).await?;
    Ok((StatusCode::CREATED, Json(project)))
}

#[utoipa::path(
    get,
    path = "/api/v1/resume-project",
    tag = "resume",
    responses(
        (status = 200, description = "Current project", body = Option<ResumeProject>),
    )
)]
async fn get_project<R: ResumeRepository + 'static>(
    State(state): State<RouteState<R>>,
) -> Result<Json<Option<ResumeProject>>, ResumeError> {
    let project = state.service.get().await?;
    Ok(Json(project))
}

#[utoipa::path(
    put,
    path = "/api/v1/resume-project",
    tag = "resume",
    request_body = UpdateResumeProjectRequest,
    responses(
        (status = 200, description = "Updated", body = ResumeProject),
    )
)]
async fn update_project<R: ResumeRepository + 'static>(
    State(state): State<RouteState<R>>,
    Json(req): Json<UpdateResumeProjectRequest>,
) -> Result<Json<ResumeProject>, ResumeError> {
    let name = req.name.as_deref().unwrap_or("");
    if name.is_empty() {
        return Err(ResumeError::InvalidGitUrl { url: "name cannot be empty".into() });
    }
    let project = state.service.update_name(name).await?;
    Ok(Json(project))
}

#[utoipa::path(
    delete,
    path = "/api/v1/resume-project",
    tag = "resume",
    responses(
        (status = 204, description = "Deleted"),
    )
)]
async fn delete_project<R: ResumeRepository + 'static>(
    State(state): State<RouteState<R>>,
) -> Result<StatusCode, ResumeError> {
    state.service.delete().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/v1/resume-project/sync",
    tag = "resume",
    responses(
        (status = 200, description = "Synced", body = ResumeProject),
    )
)]
async fn sync_project<R: ResumeRepository + 'static>(
    State(state): State<RouteState<R>>,
) -> Result<Json<ResumeProject>, ResumeError> {
    let project = state.service.sync().await?;
    Ok(Json(project))
}
```

**Step 2: Verify**

Run: `cargo check -p rara-domain-resume`

**Step 3: Commit**

```bash
git add crates/domain/resume/src/router.rs
git commit -m "refactor(resume): rewrite routes for resume-project API"
```

---

### Task 10: Update worker_state — remove typst, update resume wiring

**Files:**
- Modify: `crates/workers/src/worker_state.rs` — remove `typst_service` field, update resume route registration, remove typst tools
- Modify: `crates/workers/src/tools/services/mod.rs` — remove typst_tools module
- Delete: `crates/workers/src/tools/services/typst_tools.rs`
- Modify: `crates/workers/src/tools/services/resume_tools.rs` — simplify or remove old tools
- Modify: `crates/workers/Cargo.toml` — remove `rara-domain-typst` dependency

**Step 1: Remove typst_service from AppState struct**

In `worker_state.rs`, remove the `typst_service` field and all references:
- Remove `pub typst_service: rara_domain_typst::service::TypstService,`
- Remove `let typst_service = rara_domain_typst::wire_typst_service(...)`
- Remove typst route registration (`rara_domain_typst::router::plain_routes(...)`)
- Remove typst tool registrations

**Step 2: Update resume route registration**

Change from:
```rust
rara_domain_resume::routes::routes(self.resume_service.clone(), self.object_store.clone())
```
To:
```rust
rara_domain_resume::router::routes(self.resume_service.clone())
```

**Step 3: Remove/simplify resume tools**

Remove old resume tools (`ListResumesTool`, `GetResumeContentTool`, `AnalyzeResumeTool`) since they reference old Resume types. These can be re-added later when the new resume model is integrated with agents.

**Step 4: Remove typst tools module**

In `mod.rs`, remove `mod typst_tools` and its `pub use` line.
Delete `typst_tools.rs`.

**Step 5: Update Cargo.toml**

Remove `rara-domain-typst` from workers dependencies.

**Step 6: Verify**

Run: `cargo check -p rara-workers`

**Step 7: Commit**

```bash
git add crates/workers/
git commit -m "refactor(workers): remove typst integration, update resume wiring"
```

---

### Task 11: Remove typst domain crate

**Files:**
- Delete: entire `crates/domain/typst/` directory
- Modify: `Cargo.toml` (workspace root) — remove member + dependency

**Step 1: Remove from workspace members**

Remove `"crates/domain/typst",` from `[workspace] members`.
Remove `rara-domain-typst = { path = "crates/domain/typst" }` from `[workspace.dependencies]`.

**Step 2: Check for remaining references**

Run: `grep -r 'rara.domain.typst\|rara_domain_typst' crates/` — should only show the `crates/domain/typst/` directory itself.

**Step 3: Delete the crate directory**

```bash
rm -rf crates/domain/typst/
```

**Step 4: Verify whole workspace**

Run: `cargo check`

**Step 5: Commit**

```bash
git add -A
git commit -m "refactor: remove typst domain crate"
```

---

### Task 12: Rewrite frontend Resumes.tsx

**Files:**
- Rewrite: `web/src/pages/Resumes.tsx`
- Modify: `web/src/api/types.ts` — replace `Resume` with `ResumeProject`

**Step 1: Update API types**

Replace the `Resume` interface in `web/src/api/types.ts`:

```typescript
// Resume Project
export interface ResumeProject {
  id: string;
  name: string;
  git_url: string;
  local_path: string;
  last_synced_at: string | null;
  created_at: string;
  updated_at: string;
}

export interface SshKeyResponse {
  public_key: string;
}
```

**Step 2: Rewrite Resumes.tsx**

Simple configuration page:
- Fetch SSH public key from `GET /api/v1/settings/ssh-key`
- Fetch current project from `GET /api/v1/resume-project`
- If no project: show SSH key + setup form (name, git_url, Clone button)
- If project exists: show project info + Sync button + Remove button

**Step 3: Verify**

Run: `cd web && npm run build`

**Step 4: Commit**

```bash
git add web/
git commit -m "refactor(web): rewrite resume page as project config"
```

---

### Task 13: Remove frontend Typst pages

**Files:**
- Delete: `web/src/pages/TypstProjects.tsx`
- Delete: `web/src/pages/TypstEditor.tsx`
- Modify: `web/src/App.tsx` — remove Typst routes and imports
- Modify: `web/src/layouts/DashboardLayout.tsx` — remove Typst from `FULL_BLEED_PREFIXES`
- Modify: `web/src/api/types.ts` — remove Typst-related types
- Modify: `web/src/api/client.ts` — remove Typst-related API functions (if any)

**Step 1: Remove Typst routes from App.tsx**

Remove:
```tsx
import TypstProjects from '@/pages/TypstProjects';
import TypstEditor from '@/pages/TypstEditor';
```
And routes:
```tsx
<Route path="jobs/typst" element={<TypstProjects />} />
<Route path="jobs/typst/:projectId" element={<TypstEditor />} />
<Route path="typst" element={<Navigate to="/jobs/typst" replace />} />
```

**Step 2: Clean up DashboardLayout.tsx**

Remove `/jobs/typst/` from `FULL_BLEED_PREFIXES`.

**Step 3: Delete page files**

```bash
rm web/src/pages/TypstProjects.tsx web/src/pages/TypstEditor.tsx
```

**Step 4: Remove Typst types from types.ts**

Search for and remove any `TypstProject`, `RenderResult`, etc. interfaces.

**Step 5: Verify**

Run: `cd web && npm run build`

**Step 6: Commit**

```bash
git add web/
git commit -m "refactor(web): remove Typst pages and routes"
```

---

### Task 14: Update job-pipeline to use rara-git (optional, can defer)

**Files:**
- Modify: `crates/extensions/job-pipeline/Cargo.toml` — add `rara-git` dependency
- Modify: `crates/extensions/job-pipeline/src/tools/pipeline_tools.rs` — replace inline git2 helpers with `rara-git::GitRepo`

**Step 1: Replace `git2_create_worktree()` helper**

Replace the ~40-line `git2_create_worktree()` function (lines 721-760) with `rara_git::GitRepo::open().create_worktree()`.

**Step 2: Replace `git2_stage_and_commit()` helper**

Replace the ~45-line `git2_stage_and_commit()` function (lines 765-809) with `rara_git::GitRepo::open().stage_and_commit()`.

**Step 3: Verify**

Run: `cargo check -p rara-ext-job-pipeline`

**Step 4: Commit**

```bash
git add crates/extensions/job-pipeline/
git commit -m "refactor(pipeline): use rara-git for worktree and commit operations"
```

---

### Task 15: Final verification

**Step 1: Full backend check**

Run: `cargo check`

**Step 2: Full frontend check**

Run: `cd web && npm run build`

**Step 3: Run tests**

Run: `cargo test -p rara-git`

**Step 4: Final commit if any fixups needed**

```bash
git add -A
git commit -m "chore: fixups for resume refactor"
```
