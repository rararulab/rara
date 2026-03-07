# Symphony Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Implement `crates/symphony/` — an autonomous coding agent orchestrator that polls GitHub Issues across multiple repos, creates git worktrees, and dispatches Claude Code subprocesses.

**Architecture:** Event-loop driven orchestrator using crossbeam `SegQueue` + `tokio::sync::Notify`. Components: IssueTracker (GitHub API), WorkspaceManager (git2 worktree), CodingAgent trait (subprocess), Orchestrator (event loop + state machine).

**Tech Stack:** Rust, git2, reqwest, crossbeam-queue, tokio, snafu, bon, humantime-serde

---

### Task 1: Crate Scaffolding + Error Types

**Files:**
- Create: `crates/symphony/Cargo.toml`
- Create: `crates/symphony/src/lib.rs`
- Create: `crates/symphony/src/error.rs`
- Modify: `Cargo.toml` (workspace root)

**Step 1: Add to workspace members**

In root `Cargo.toml`, add `"crates/symphony"` to the `[workspace] members` list (around line 104). Also add the workspace dependency alias in `[workspace.dependencies]`:

```toml
rara-symphony = { path = "crates/symphony" }
```

**Step 2: Create Cargo.toml**

Create `crates/symphony/Cargo.toml`:

```toml
[package]
name = "rara-symphony"
version.workspace = true
edition.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
homepage.workspace = true
readme.workspace = true
keywords.workspace = true
categories.workspace = true
description = "Autonomous coding agent orchestrator for GitHub Issues"

[dependencies]
async-trait.workspace = true
bon.workspace = true
chrono = { workspace = true, features = ["serde"] }
crossbeam-queue.workspace = true
git2.workspace = true
humantime-serde = "1"
reqwest = { workspace = true, features = ["json"] }
serde.workspace = true
serde_json.workspace = true
snafu.workspace = true
tokio = { workspace = true, features = ["process", "time", "sync"] }
tokio-util.workspace = true
tracing.workspace = true

[dev-dependencies]
tempfile.workspace = true
tokio = { workspace = true, features = ["test-util", "macros"] }

[lints]
workspace = true
```

**Step 3: Create error.rs**

```rust
use snafu::Snafu;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum SymphonyError {
    #[snafu(display("github API error: {message}"))]
    GitHub { message: String },

    #[snafu(display("git error: {source}"))]
    Git { source: git2::Error },

    #[snafu(display("workspace error: {message}"))]
    Workspace { message: String },

    #[snafu(display("agent error: {message}"))]
    Agent { message: String },

    #[snafu(display("hook failed: {hook} — {message}"))]
    Hook { hook: String, message: String },

    #[snafu(display("config error: {message}"))]
    Config { message: String },

    #[snafu(display("IO error: {source}"))]
    Io { source: std::io::Error },
}

pub type Result<T, E = SymphonyError> = std::result::Result<T, E>;
```

**Step 4: Create lib.rs**

```rust
pub mod error;

pub use error::{SymphonyError, Result};
```

**Step 5: Verify it compiles**

Run: `cargo check -p rara-symphony`
Expected: compiles with no errors

**Step 6: Commit**

```
feat(symphony): initialize crate scaffolding with error types (#N)
```

---

### Task 2: Config Types

**Files:**
- Create: `crates/symphony/src/config.rs`
- Modify: `crates/symphony/src/lib.rs`

**Step 1: Create config.rs**

```rust
use std::path::PathBuf;
use std::time::Duration;

use bon::Builder;
use serde::Deserialize;

#[derive(Debug, Clone, Builder, Deserialize)]
pub struct SymphonyConfig {
    #[serde(default)]
    pub enabled: bool,

    #[serde(deserialize_with = "humantime_serde::deserialize")]
    pub poll_interval: Duration,

    pub max_concurrent_agents: usize,

    #[serde(deserialize_with = "humantime_serde::deserialize")]
    pub stall_timeout: Duration,

    #[serde(deserialize_with = "humantime_serde::deserialize")]
    pub max_retry_backoff: Duration,

    #[serde(default = "default_workflow_file")]
    pub workflow_file: String,

    pub agent: AgentConfig,

    pub repos: Vec<RepoConfig>,
}

#[derive(Debug, Clone, Builder, Deserialize)]
pub struct AgentConfig {
    #[serde(default = "default_agent_command")]
    pub command: String,

    #[serde(default)]
    pub args: Vec<String>,

    #[serde(default)]
    pub allowed_tools: Vec<String>,

    #[serde(deserialize_with = "humantime_serde::deserialize")]
    pub turn_timeout: Duration,
}

#[derive(Debug, Clone, Builder, Deserialize)]
pub struct RepoConfig {
    pub name: String,

    pub url: String,

    pub repo_path: PathBuf,

    pub workspace_root: PathBuf,

    #[serde(default = "default_active_labels")]
    pub active_labels: Vec<String>,

    pub max_concurrent_agents: Option<usize>,

    pub workflow_file: Option<String>,

    #[serde(default)]
    pub hooks: HooksConfig,
}

#[derive(Debug, Clone, Default, Builder, Deserialize)]
pub struct HooksConfig {
    pub after_create: Option<String>,
    pub before_run: Option<String>,
    pub after_run: Option<String>,
    pub before_remove: Option<String>,
}

fn default_workflow_file() -> String {
    "WORKFLOW.md".to_string()
}

fn default_agent_command() -> String {
    "claude".to_string()
}

fn default_active_labels() -> Vec<String> {
    vec!["symphony:ready".to_string()]
}
```

**Step 2: Add to lib.rs**

```rust
pub mod config;
pub mod error;

pub use config::SymphonyConfig;
pub use error::{SymphonyError, Result};
```

**Step 3: Verify**

Run: `cargo check -p rara-symphony`

**Step 4: Commit**

```
feat(symphony): add config types for multi-repo orchestration (#N)
```

---

### Task 3: Event System + EventQueue

**Files:**
- Create: `crates/symphony/src/event.rs`
- Create: `crates/symphony/src/queue.rs`
- Modify: `crates/symphony/src/lib.rs`

**Step 1: Create event.rs**

```rust
use std::path::PathBuf;

use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct TrackedIssue {
    pub id: String,
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub body: String,
    pub labels: Vec<String>,
    pub priority: Option<u32>,
    pub state: IssueState,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueState {
    Active,
    Terminal,
}

#[derive(Debug, Clone)]
pub struct WorkspaceInfo {
    pub path: PathBuf,
    pub branch: String,
    pub created_now: bool,
}

#[derive(Debug)]
pub enum SymphonyEvent {
    // Timer
    PollTick,
    StallCheck,

    // Issue lifecycle
    IssueDiscovered(TrackedIssue),
    IssueStateChanged {
        issue_id: String,
        new_state: IssueState,
    },

    // Agent lifecycle
    AgentCompleted {
        issue_id: String,
        output: String,
    },
    AgentFailed {
        issue_id: String,
        error: String,
        exit_code: Option<i32>,
    },
    AgentStalled {
        issue_id: String,
    },

    // Workspace
    WorkspaceCleaned {
        issue_id: String,
    },

    // Retry
    RetryReady {
        issue_id: String,
        attempt: u32,
    },

    // System
    Shutdown,
}
```

**Step 2: Create queue.rs**

```rust
use std::sync::Arc;

use crossbeam_queue::SegQueue;
use tokio::sync::Notify;

use crate::event::SymphonyEvent;

#[derive(Clone)]
pub struct EventQueue {
    queue: Arc<SegQueue<SymphonyEvent>>,
    notify: Arc<Notify>,
}

impl EventQueue {
    pub fn new() -> Self {
        Self {
            queue: Arc::new(SegQueue::new()),
            notify: Arc::new(Notify::new()),
        }
    }

    pub fn push(&self, event: SymphonyEvent) {
        self.queue.push(event);
        self.notify.notify_one();
    }

    pub async fn pop(&self) -> SymphonyEvent {
        loop {
            if let Some(event) = self.queue.pop() {
                return event;
            }
            self.notify.notified().await;
        }
    }

    pub fn schedule_after(&self, event: SymphonyEvent, delay: std::time::Duration) {
        let this = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            this.push(event);
        });
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}
```

**Step 3: Write test for EventQueue**

Add to bottom of `queue.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn push_and_pop() {
        let q = EventQueue::new();
        q.push(SymphonyEvent::PollTick);
        let event = q.pop().await;
        assert!(matches!(event, SymphonyEvent::PollTick));
    }

    #[tokio::test]
    async fn pop_waits_for_push() {
        let q = EventQueue::new();
        let q2 = q.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            q2.push(SymphonyEvent::Shutdown);
        });
        let event = q.pop().await;
        assert!(matches!(event, SymphonyEvent::Shutdown));
    }

    #[tokio::test]
    async fn schedule_after_delivers() {
        let q = EventQueue::new();
        q.schedule_after(SymphonyEvent::PollTick, Duration::from_millis(50));
        let event = q.pop().await;
        assert!(matches!(event, SymphonyEvent::PollTick));
    }
}
```

**Step 4: Update lib.rs**

```rust
pub mod config;
pub mod error;
pub mod event;
pub mod queue;

pub use config::SymphonyConfig;
pub use error::{SymphonyError, Result};
pub use event::{SymphonyEvent, TrackedIssue, IssueState, WorkspaceInfo};
pub use queue::EventQueue;
```

**Step 5: Run tests**

Run: `cargo test -p rara-symphony`
Expected: 3 tests pass

**Step 6: Commit**

```
feat(symphony): add event types and crossbeam-based EventQueue (#N)
```

---

### Task 4: IssueTracker (GitHub API)

**Files:**
- Create: `crates/symphony/src/tracker.rs`
- Modify: `crates/symphony/src/lib.rs`

**Step 1: Create tracker.rs**

```rust
use async_trait::async_trait;
use snafu::ResultExt;
use tracing::{debug, warn};

use crate::config::RepoConfig;
use crate::error::{self, Result};
use crate::event::{IssueState, TrackedIssue};

#[async_trait]
pub trait IssueTracker: Send + Sync {
    async fn fetch_active_issues(&self) -> Result<Vec<TrackedIssue>>;
    async fn fetch_issue_state(&self, repo: &str, number: u64) -> Result<IssueState>;
}

pub struct GitHubIssueTracker {
    repos: Vec<RepoConfig>,
    client: reqwest::Client,
    token: Option<String>,
}

impl GitHubIssueTracker {
    pub fn new(repos: Vec<RepoConfig>, token: Option<String>) -> Self {
        Self {
            repos,
            client: reqwest::Client::new(),
            token,
        }
    }

    async fn fetch_repo_issues(&self, repo: &RepoConfig) -> Result<Vec<TrackedIssue>> {
        // Use label filter in query: GET /repos/{owner}/{repo}/issues?labels=symphony:ready&state=open
        let labels = repo.active_labels.join(",");
        let url = format!(
            "https://api.github.com/repos/{}/issues?state=open&labels={}&per_page=100",
            repo.url.trim_start_matches("https://github.com/"),
            labels,
        );

        let mut request = self.client
            .get(&url)
            .header("User-Agent", "rara-symphony")
            .header("Accept", "application/vnd.github+json");

        if let Some(ref token) = self.token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }

        let response = request.send().await.map_err(|e| {
            error::GitHubSnafu { message: e.to_string() }.build()
        })?;

        if !response.status().is_success() {
            return Err(error::GitHubSnafu {
                message: format!("HTTP {}: {}", response.status(), repo.name),
            }.build());
        }

        let items: Vec<serde_json::Value> = response.json().await.map_err(|e| {
            error::GitHubSnafu { message: e.to_string() }.build()
        })?;

        let mut issues = Vec::new();
        for item in items {
            // Skip pull requests (GitHub Issues API includes PRs)
            if item.get("pull_request").is_some() {
                continue;
            }

            let number = item["number"].as_u64().unwrap_or(0);
            let title = item["title"].as_str().unwrap_or("").to_string();
            let body = item["body"].as_str().unwrap_or("").to_string();

            let labels: Vec<String> = item["labels"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|l| l["name"].as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            let priority = derive_priority(&labels);

            let created_at = item["created_at"]
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .unwrap_or_else(chrono::Utc::now);

            let repo_slug = repo.url.trim_start_matches("https://github.com/");

            issues.push(TrackedIssue {
                id: format!("{repo_slug}#{number}"),
                repo: repo_slug.to_string(),
                number,
                title,
                body,
                labels,
                priority,
                state: IssueState::Active,
                created_at,
            });
        }

        debug!(repo = %repo.name, count = issues.len(), "fetched issues");
        Ok(issues)
    }
}

#[async_trait]
impl IssueTracker for GitHubIssueTracker {
    async fn fetch_active_issues(&self) -> Result<Vec<TrackedIssue>> {
        let mut all = Vec::new();
        for repo in &self.repos {
            match self.fetch_repo_issues(repo).await {
                Ok(issues) => all.extend(issues),
                Err(e) => {
                    warn!(repo = %repo.name, error = %e, "failed to fetch issues, skipping repo");
                }
            }
        }

        // Sort: priority asc (None last) → created_at oldest → number
        all.sort_by(|a, b| {
            let pa = a.priority.unwrap_or(u32::MAX);
            let pb = b.priority.unwrap_or(u32::MAX);
            pa.cmp(&pb)
                .then(a.created_at.cmp(&b.created_at))
                .then(a.number.cmp(&b.number))
        });

        Ok(all)
    }

    async fn fetch_issue_state(&self, repo: &str, number: u64) -> Result<IssueState> {
        let url = format!("https://api.github.com/repos/{repo}/issues/{number}");

        let mut request = self.client
            .get(&url)
            .header("User-Agent", "rara-symphony")
            .header("Accept", "application/vnd.github+json");

        if let Some(ref token) = self.token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }

        let response = request.send().await.map_err(|e| {
            error::GitHubSnafu { message: e.to_string() }.build()
        })?;

        let item: serde_json::Value = response.json().await.map_err(|e| {
            error::GitHubSnafu { message: e.to_string() }.build()
        })?;

        let state_str = item["state"].as_str().unwrap_or("open");
        if state_str == "closed" {
            Ok(IssueState::Terminal)
        } else {
            Ok(IssueState::Active)
        }
    }
}

/// Derive numeric priority from labels like "priority:1", "priority:high", etc.
fn derive_priority(labels: &[String]) -> Option<u32> {
    for label in labels {
        if let Some(suffix) = label.strip_prefix("priority:") {
            match suffix {
                "critical" | "1" => return Some(1),
                "high" | "2" => return Some(2),
                "medium" | "3" => return Some(3),
                "low" | "4" => return Some(4),
                _ => {}
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_from_labels() {
        assert_eq!(derive_priority(&["priority:1".into()]), Some(1));
        assert_eq!(derive_priority(&["priority:high".into()]), Some(2));
        assert_eq!(derive_priority(&["priority:low".into()]), Some(4));
        assert_eq!(derive_priority(&["bug".into()]), None);
    }

    #[test]
    fn sort_issues_priority_then_age() {
        let make = |id: &str, pri: Option<u32>, hours_ago: i64| TrackedIssue {
            id: id.to_string(),
            repo: "test/repo".into(),
            number: 1,
            title: String::new(),
            body: String::new(),
            labels: vec![],
            priority: pri,
            state: IssueState::Active,
            created_at: chrono::Utc::now() - chrono::Duration::hours(hours_ago),
        };

        let mut issues = vec![
            make("c", None, 10),
            make("a", Some(1), 5),
            make("b", Some(2), 20),
        ];

        issues.sort_by(|a, b| {
            let pa = a.priority.unwrap_or(u32::MAX);
            let pb = b.priority.unwrap_or(u32::MAX);
            pa.cmp(&pb)
                .then(a.created_at.cmp(&b.created_at))
                .then(a.number.cmp(&b.number))
        });

        assert_eq!(issues[0].id, "a"); // priority 1
        assert_eq!(issues[1].id, "b"); // priority 2
        assert_eq!(issues[2].id, "c"); // no priority (last)
    }
}
```

**Step 2: Update lib.rs — add `pub mod tracker;` and re-export**

**Step 3: Run tests**

Run: `cargo test -p rara-symphony`

**Step 4: Commit**

```
feat(symphony): add GitHub issue tracker with priority sorting (#N)
```

---

### Task 5: WorkspaceManager (git2 worktree)

**Files:**
- Create: `crates/symphony/src/workspace.rs`
- Modify: `crates/symphony/src/lib.rs`

**Step 1: Create workspace.rs**

```rust
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use git2::Repository;
use snafu::ResultExt;
use tracing::{debug, info, warn};

use crate::config::{HooksConfig, RepoConfig};
use crate::error::{self, Result};
use crate::event::WorkspaceInfo;

pub struct WorkspaceManager {
    repos: HashMap<String, RepoWorkspaceConfig>,
}

struct RepoWorkspaceConfig {
    repo_path: PathBuf,
    workspace_root: PathBuf,
    hooks: HooksConfig,
}

impl WorkspaceManager {
    pub fn new(repo_configs: &[RepoConfig]) -> Result<Self> {
        let mut repos = HashMap::new();
        for rc in repo_configs {
            repos.insert(rc.name.clone(), RepoWorkspaceConfig {
                repo_path: rc.repo_path.clone(),
                workspace_root: rc.workspace_root.clone(),
                hooks: rc.hooks.clone(),
            });
        }
        Ok(Self { repos })
    }

    pub fn ensure_worktree(
        &self,
        repo_name: &str,
        issue_number: u64,
        issue_title: &str,
    ) -> Result<WorkspaceInfo> {
        let config = self.repos.get(repo_name).ok_or_else(|| {
            error::WorkspaceSnafu {
                message: format!("unknown repo: {repo_name}"),
            }.build()
        })?;

        let branch = worktree_branch_name(issue_number, issue_title);
        let worktree_path = config.workspace_root.join(&branch);

        // If worktree already exists, reuse it
        if worktree_path.exists() {
            debug!(path = %worktree_path.display(), "reusing existing worktree");
            return Ok(WorkspaceInfo {
                path: worktree_path,
                branch,
                created_now: false,
            });
        }

        // Ensure workspace root exists
        std::fs::create_dir_all(&config.workspace_root).context(error::IoSnafu)?;

        let repo = Repository::open(&config.repo_path).context(error::GitSnafu)?;

        // Find HEAD commit for branch base
        let head = repo.head().context(error::GitSnafu)?;
        let commit = head.peel_to_commit().context(error::GitSnafu)?;

        // Create branch
        repo.branch(&branch, &commit, false).context(error::GitSnafu)?;

        // Create worktree
        let reference = repo
            .find_branch(&branch, git2::BranchType::Local)
            .context(error::GitSnafu)?;

        repo.worktree(
            &branch,
            &worktree_path,
            Some(
                git2::WorktreeAddOptions::new()
                    .reference(Some(reference.get())),
            ),
        )
        .context(error::GitSnafu)?;

        info!(
            path = %worktree_path.display(),
            branch = %branch,
            "created worktree"
        );

        Ok(WorkspaceInfo {
            path: worktree_path,
            branch,
            created_now: true,
        })
    }

    pub fn cleanup_worktree(&self, repo_name: &str, workspace: &WorkspaceInfo) -> Result<()> {
        let config = self.repos.get(repo_name).ok_or_else(|| {
            error::WorkspaceSnafu {
                message: format!("unknown repo: {repo_name}"),
            }.build()
        })?;

        let repo = Repository::open(&config.repo_path).context(error::GitSnafu)?;

        // Prune worktree
        if let Ok(wt) = repo.find_worktree(&workspace.branch) {
            if wt.validate().is_ok() {
                repo.worktree(&workspace.branch, &workspace.path, None)
                    .ok(); // best-effort
            }
        }

        // Remove worktree directory
        if workspace.path.exists() {
            std::fs::remove_dir_all(&workspace.path).context(error::IoSnafu)?;
        }

        // Prune worktree reference
        if let Ok(wt) = repo.find_worktree(&workspace.branch) {
            wt.prune(Some(
                git2::WorktreePruneOptions::new()
                    .valid(true)
                    .working_tree(true),
            ))
            .context(error::GitSnafu)?;
        }

        // Delete branch
        if let Ok(mut branch) = repo.find_branch(&workspace.branch, git2::BranchType::Local) {
            branch.delete().context(error::GitSnafu)?;
        }

        info!(
            branch = %workspace.branch,
            "cleaned up worktree and branch"
        );

        Ok(())
    }

    pub fn hooks_for(&self, repo_name: &str) -> Option<&HooksConfig> {
        self.repos.get(repo_name).map(|c| &c.hooks)
    }
}

pub async fn run_hook(hook_script: &str, cwd: &Path) -> Result<()> {
    let output = tokio::process::Command::new("sh")
        .arg("-lc")
        .arg(hook_script)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context(error::IoSnafu)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(error::HookSnafu {
            hook: hook_script.to_string(),
            message: stderr.to_string(),
        }.build());
    }

    Ok(())
}

fn worktree_branch_name(issue_number: u64, title: &str) -> String {
    let sanitized: String = title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");

    let short = if sanitized.len() > 40 {
        &sanitized[..40]
    } else {
        &sanitized
    };

    format!("issue-{issue_number}-{short}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_name_sanitization() {
        assert_eq!(
            worktree_branch_name(42, "feat: Add Symphony support!"),
            "issue-42-feat-add-symphony-support"
        );
    }

    #[test]
    fn branch_name_truncation() {
        let long_title = "a".repeat(100);
        let name = worktree_branch_name(1, &long_title);
        assert!(name.len() <= 50);
    }

    #[test]
    fn worktree_create_and_cleanup() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_path = tmp.path().join("repo");

        // Init a bare-enough repo with at least one commit
        let repo = Repository::init(&repo_path).unwrap();
        {
            let mut index = repo.index().unwrap();
            let tree_id = index.write_tree().unwrap();
            let tree = repo.find_tree(tree_id).unwrap();
            let sig = git2::Signature::now("test", "test@test.com").unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[]).unwrap();
        }

        let workspace_root = tmp.path().join("workspaces");
        let configs = vec![crate::config::RepoConfig::builder()
            .name("test")
            .url("https://github.com/test/repo")
            .repo_path(repo_path)
            .workspace_root(workspace_root.clone())
            .active_labels(vec!["symphony:ready".into()])
            .hooks(HooksConfig::default())
            .build()];

        let mgr = WorkspaceManager::new(&configs).unwrap();

        // Create
        let info = mgr.ensure_worktree("test", 42, "test feature").unwrap();
        assert!(info.created_now);
        assert!(info.path.exists());

        // Reuse
        let info2 = mgr.ensure_worktree("test", 42, "test feature").unwrap();
        assert!(!info2.created_now);

        // Cleanup
        mgr.cleanup_worktree("test", &info).unwrap();
        assert!(!info.path.exists());
    }
}
```

**Step 2: Update lib.rs — add `pub mod workspace;`**

**Step 3: Run tests**

Run: `cargo test -p rara-symphony`

**Step 4: Commit**

```
feat(symphony): add WorkspaceManager with git2 worktree lifecycle (#N)
```

---

### Task 6: CodingAgent Trait + ClaudeCodeAgent

**Files:**
- Create: `crates/symphony/src/agent.rs`
- Modify: `crates/symphony/src/lib.rs`

**Step 1: Create agent.rs**

```rust
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Instant;

use async_trait::async_trait;
use snafu::ResultExt;
use tokio::process::{Child, Command};

use crate::config::AgentConfig;
use crate::error::{self, Result};
use crate::event::{TrackedIssue, WorkspaceInfo};

pub struct AgentTask {
    pub issue: TrackedIssue,
    pub prompt: String,
    pub workflow_content: Option<String>,
}

pub struct AgentHandle {
    pub child: Child,
    pub started_at: Instant,
}

#[async_trait]
pub trait CodingAgent: Send + Sync {
    async fn start(
        &self,
        task: &AgentTask,
        workspace: &WorkspaceInfo,
    ) -> Result<AgentHandle>;
}

pub struct ClaudeCodeAgent {
    config: AgentConfig,
}

impl ClaudeCodeAgent {
    pub fn new(config: AgentConfig) -> Self {
        Self { config }
    }

    fn build_prompt(&self, task: &AgentTask) -> String {
        let mut prompt = format!(
            "You are working on issue #{}: {}\n\n## Issue Description\n{}\n",
            task.issue.number, task.issue.title, task.issue.body,
        );

        if let Some(ref workflow) = task.workflow_content {
            prompt.push_str(&format!("\n## Workflow Guidelines\n{workflow}\n"));
        }

        prompt.push_str(
            "\n## Instructions\n\
             - Work in the current directory (a git worktree)\n\
             - Commit your changes with conventional commit messages\n\
             - Include the issue number in commit messages\n\
             - Create a PR when done using `gh pr create`\n"
        );

        prompt
    }
}

#[async_trait]
impl CodingAgent for ClaudeCodeAgent {
    async fn start(
        &self,
        task: &AgentTask,
        workspace: &WorkspaceInfo,
    ) -> Result<AgentHandle> {
        let prompt = self.build_prompt(task);

        let mut cmd = Command::new(&self.config.command);

        // Add configured args (e.g. ["-p"])
        for arg in &self.config.args {
            cmd.arg(arg);
        }

        cmd.arg(&prompt);
        cmd.current_dir(&workspace.path);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        // Add allowed tools if configured
        for tool in &self.config.allowed_tools {
            cmd.arg("--allowedTools");
            cmd.arg(tool);
        }

        let child = cmd.spawn().context(error::IoSnafu)?;

        Ok(AgentHandle {
            child,
            started_at: Instant::now(),
        })
    }
}
```

**Step 2: Update lib.rs — add `pub mod agent;`**

**Step 3: Verify**

Run: `cargo check -p rara-symphony`

**Step 4: Commit**

```
feat(symphony): add CodingAgent trait with ClaudeCodeAgent implementation (#N)
```

---

### Task 7: Orchestrator (Event Loop + Handlers)

**Files:**
- Create: `crates/symphony/src/orchestrator.rs`
- Modify: `crates/symphony/src/lib.rs`

**Step 1: Create orchestrator.rs**

```rust
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};

use tracing::{debug, error, info, warn, instrument};

use crate::agent::{AgentHandle, AgentTask, CodingAgent};
use crate::config::SymphonyConfig;
use crate::error::Result;
use crate::event::{IssueState, SymphonyEvent, TrackedIssue, WorkspaceInfo};
use crate::queue::EventQueue;
use crate::tracker::IssueTracker;
use crate::workspace::{self, WorkspaceManager};

pub struct Orchestrator {
    tracker: Box<dyn IssueTracker>,
    workspace_mgr: WorkspaceManager,
    agent: Box<dyn CodingAgent>,
    config: SymphonyConfig,
    queue: EventQueue,

    running: HashMap<String, RunState>,
    claimed: HashSet<String>,
    retries: HashMap<String, RetryEntry>,
}

struct RunState {
    issue: TrackedIssue,
    workspace: WorkspaceInfo,
    started_at: Instant,
    last_activity: Instant,
}

struct RetryEntry {
    attempt: u32,
}

impl Orchestrator {
    pub fn new(
        tracker: Box<dyn IssueTracker>,
        workspace_mgr: WorkspaceManager,
        agent: Box<dyn CodingAgent>,
        config: SymphonyConfig,
    ) -> Self {
        Self {
            tracker,
            workspace_mgr,
            agent,
            config,
            queue: EventQueue::new(),
            running: HashMap::new(),
            claimed: HashSet::new(),
            retries: HashMap::new(),
        }
    }

    pub fn queue(&self) -> &EventQueue {
        &self.queue
    }

    #[instrument(skip(self), name = "symphony::run")]
    pub async fn run(&mut self) -> Result<()> {
        self.queue.push(SymphonyEvent::PollTick);
        self.queue.schedule_after(
            SymphonyEvent::StallCheck,
            self.config.stall_timeout / 2,
        );

        loop {
            let event = self.queue.pop().await;
            match event {
                SymphonyEvent::PollTick => {
                    if let Err(e) = self.handle_poll_tick().await {
                        error!(error = %e, "poll tick failed");
                    }
                    self.queue.schedule_after(
                        SymphonyEvent::PollTick,
                        self.config.poll_interval,
                    );
                }

                SymphonyEvent::StallCheck => {
                    self.handle_stall_check();
                    self.queue.schedule_after(
                        SymphonyEvent::StallCheck,
                        self.config.stall_timeout / 2,
                    );
                }

                SymphonyEvent::IssueDiscovered(issue) => {
                    if let Err(e) = self.handle_dispatch(issue).await {
                        error!(error = %e, "dispatch failed");
                    }
                }

                SymphonyEvent::AgentCompleted { issue_id, output } => {
                    self.handle_agent_completed(&issue_id, &output).await;
                }

                SymphonyEvent::AgentFailed { issue_id, error, exit_code } => {
                    self.handle_agent_failed(&issue_id, &error, exit_code);
                }

                SymphonyEvent::AgentStalled { issue_id } => {
                    self.handle_agent_stalled(&issue_id);
                }

                SymphonyEvent::IssueStateChanged { issue_id, new_state } => {
                    self.handle_state_changed(&issue_id, new_state).await;
                }

                SymphonyEvent::RetryReady { issue_id, attempt } => {
                    self.handle_retry(&issue_id, attempt).await;
                }

                SymphonyEvent::WorkspaceCleaned { issue_id } => {
                    self.running.remove(&issue_id);
                    self.claimed.remove(&issue_id);
                    self.retries.remove(&issue_id);
                }

                SymphonyEvent::Shutdown => {
                    info!("shutting down orchestrator");
                    break;
                }
            }
        }
        Ok(())
    }

    async fn handle_poll_tick(&self) -> Result<()> {
        debug!("polling for issues");
        let issues = self.tracker.fetch_active_issues().await?;
        let eligible: Vec<_> = issues
            .into_iter()
            .filter(|i| !self.running.contains_key(&i.id) && !self.claimed.contains(&i.id))
            .collect();

        debug!(count = eligible.len(), "eligible issues found");
        for issue in eligible {
            self.queue.push(SymphonyEvent::IssueDiscovered(issue));
        }
        Ok(())
    }

    async fn handle_dispatch(&mut self, issue: TrackedIssue) -> Result<()> {
        if !self.has_global_slots() {
            debug!(issue = %issue.id, "no global slots available, skipping");
            return Ok(());
        }
        if !self.has_repo_slots(&issue.repo) {
            debug!(issue = %issue.id, "no repo slots available, skipping");
            return Ok(());
        }
        if self.claimed.contains(&issue.id) {
            return Ok(());
        }

        self.claimed.insert(issue.id.clone());
        info!(issue = %issue.id, title = %issue.title, "dispatching issue");

        // Find repo name from issue.repo slug
        let repo_name = self.repo_name_for(&issue.repo);

        // Create worktree
        let workspace = self.workspace_mgr.ensure_worktree(
            &repo_name,
            issue.number,
            &issue.title,
        )?;

        // Run hooks
        if workspace.created_now {
            if let Some(hooks) = self.workspace_mgr.hooks_for(&repo_name) {
                if let Some(ref script) = hooks.after_create {
                    if let Err(e) = workspace::run_hook(script, &workspace.path).await {
                        warn!(issue = %issue.id, error = %e, "after_create hook failed");
                    }
                }
            }
        }
        if let Some(hooks) = self.workspace_mgr.hooks_for(&repo_name) {
            if let Some(ref script) = hooks.before_run {
                if let Err(e) = workspace::run_hook(script, &workspace.path).await {
                    warn!(issue = %issue.id, error = %e, "before_run hook failed");
                }
            }
        }

        // Load WORKFLOW.md if exists
        let workflow_file = self.workflow_file_for(&repo_name);
        let workflow_content = tokio::fs::read_to_string(workspace.path.join(&workflow_file))
            .await
            .ok();

        // Build task and start agent
        let task = AgentTask {
            issue: issue.clone(),
            prompt: String::new(), // built by agent
            workflow_content,
        };

        let handle = self.agent.start(&task, &workspace).await?;

        // Spawn process watcher
        let queue = self.queue.clone();
        let id = issue.id.clone();
        let child = handle.child;
        tokio::spawn(async move {
            let output = child.wait_with_output().await;
            let event = match output {
                Ok(out) if out.status.success() => SymphonyEvent::AgentCompleted {
                    issue_id: id,
                    output: String::from_utf8_lossy(&out.stdout).to_string(),
                },
                Ok(out) => SymphonyEvent::AgentFailed {
                    issue_id: id,
                    error: String::from_utf8_lossy(&out.stderr).to_string(),
                    exit_code: out.status.code(),
                },
                Err(e) => SymphonyEvent::AgentFailed {
                    issue_id: id,
                    error: e.to_string(),
                    exit_code: None,
                },
            };
            queue.push(event);
        });

        self.running.insert(issue.id.clone(), RunState {
            issue,
            workspace,
            started_at: Instant::now(),
            last_activity: Instant::now(),
        });

        Ok(())
    }

    async fn handle_agent_completed(&mut self, issue_id: &str, output: &str) {
        info!(issue = %issue_id, "agent completed successfully");

        if let Some(state) = self.running.get(issue_id) {
            let repo_name = self.repo_name_for(&state.issue.repo);

            // Run after_run hook
            if let Some(hooks) = self.workspace_mgr.hooks_for(&repo_name) {
                if let Some(ref script) = hooks.after_run {
                    let _ = workspace::run_hook(script, &state.workspace.path).await;
                }
            }

            // Cleanup worktree
            if let Err(e) = self.workspace_mgr.cleanup_worktree(&repo_name, &state.workspace) {
                warn!(issue = %issue_id, error = %e, "worktree cleanup failed");
            }
        }

        self.queue.push(SymphonyEvent::WorkspaceCleaned {
            issue_id: issue_id.to_string(),
        });
    }

    fn handle_agent_failed(&mut self, issue_id: &str, error_msg: &str, exit_code: Option<i32>) {
        warn!(
            issue = %issue_id,
            exit_code = ?exit_code,
            error = %error_msg,
            "agent failed"
        );

        let attempt = self.retries.get(issue_id).map_or(1, |r| r.attempt + 1);
        self.retries.insert(issue_id.to_string(), RetryEntry { attempt });

        let delay = self.compute_backoff(attempt);
        info!(issue = %issue_id, attempt, delay_ms = delay.as_millis(), "scheduling retry");

        self.queue.schedule_after(
            SymphonyEvent::RetryReady {
                issue_id: issue_id.to_string(),
                attempt,
            },
            delay,
        );
    }

    fn handle_agent_stalled(&mut self, issue_id: &str) {
        warn!(issue = %issue_id, "agent stalled, treating as failure");
        self.handle_agent_failed(issue_id, "stall timeout exceeded", None);
    }

    async fn handle_state_changed(&mut self, issue_id: &str, new_state: IssueState) {
        if new_state == IssueState::Terminal {
            info!(issue = %issue_id, "issue closed externally, cleaning up");

            if let Some(state) = self.running.get(issue_id) {
                let repo_name = self.repo_name_for(&state.issue.repo);
                let _ = self.workspace_mgr.cleanup_worktree(&repo_name, &state.workspace);
            }

            self.queue.push(SymphonyEvent::WorkspaceCleaned {
                issue_id: issue_id.to_string(),
            });
        }
    }

    async fn handle_retry(&mut self, issue_id: &str, attempt: u32) {
        debug!(issue = %issue_id, attempt, "retry ready");

        // Re-check issue state before retrying
        if let Some(state) = self.running.remove(issue_id) {
            self.claimed.remove(issue_id);

            match self.tracker.fetch_issue_state(&state.issue.repo, state.issue.number).await {
                Ok(IssueState::Active) => {
                    // Re-dispatch
                    self.queue.push(SymphonyEvent::IssueDiscovered(state.issue));
                }
                Ok(IssueState::Terminal) => {
                    info!(issue = %issue_id, "issue now closed, skipping retry");
                    let _ = self.workspace_mgr.cleanup_worktree(
                        &self.repo_name_for(&state.issue.repo),
                        &state.workspace,
                    );
                }
                Err(e) => {
                    warn!(issue = %issue_id, error = %e, "failed to check issue state for retry");
                }
            }
        }
    }

    fn handle_stall_check(&mut self) {
        let stall_timeout = self.config.stall_timeout;
        let stalled: Vec<String> = self.running
            .iter()
            .filter(|(_, state)| state.last_activity.elapsed() > stall_timeout)
            .map(|(id, _)| id.clone())
            .collect();

        for id in stalled {
            self.queue.push(SymphonyEvent::AgentStalled { issue_id: id });
        }
    }

    fn has_global_slots(&self) -> bool {
        self.running.len() < self.config.max_concurrent_agents
    }

    fn has_repo_slots(&self, repo: &str) -> bool {
        let repo_config = self.config.repos.iter().find(|r| {
            r.url.trim_start_matches("https://github.com/") == repo
        });

        if let Some(rc) = repo_config {
            if let Some(max) = rc.max_concurrent_agents {
                let count = self.running.values().filter(|s| s.issue.repo == repo).count();
                return count < max;
            }
        }
        true
    }

    fn repo_name_for(&self, repo_slug: &str) -> String {
        self.config
            .repos
            .iter()
            .find(|r| r.url.trim_start_matches("https://github.com/") == repo_slug)
            .map(|r| r.name.clone())
            .unwrap_or_else(|| repo_slug.to_string())
    }

    fn workflow_file_for(&self, repo_name: &str) -> String {
        self.config
            .repos
            .iter()
            .find(|r| r.name == repo_name)
            .and_then(|r| r.workflow_file.clone())
            .unwrap_or_else(|| self.config.workflow_file.clone())
    }

    fn compute_backoff(&self, attempt: u32) -> Duration {
        let base = Duration::from_secs(10);
        let delay = base.saturating_mul(2u32.saturating_pow(attempt.saturating_sub(1)));
        std::cmp::min(delay, self.config.max_retry_backoff)
    }
}
```

**Step 2: Update lib.rs — add `pub mod orchestrator;`**

**Step 3: Verify**

Run: `cargo check -p rara-symphony`

**Step 4: Commit**

```
feat(symphony): add Orchestrator with event-loop dispatch and retry (#N)
```

---

### Task 8: SymphonyService Entry Point + App Integration

**Files:**
- Create: `crates/symphony/src/service.rs`
- Modify: `crates/symphony/src/lib.rs`
- Modify: `crates/app/Cargo.toml` (add rara-symphony dependency)
- Modify: `crates/app/src/lib.rs` (add SymphonyConfig + spawn)

**Step 1: Create service.rs**

```rust
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::agent::ClaudeCodeAgent;
use crate::config::SymphonyConfig;
use crate::error::Result;
use crate::orchestrator::Orchestrator;
use crate::tracker::GitHubIssueTracker;
use crate::workspace::WorkspaceManager;

pub struct SymphonyService {
    config: SymphonyConfig,
    shutdown: CancellationToken,
    github_token: Option<String>,
}

impl SymphonyService {
    pub fn new(
        config: SymphonyConfig,
        shutdown: CancellationToken,
        github_token: Option<String>,
    ) -> Self {
        Self {
            config,
            shutdown,
            github_token,
        }
    }

    pub async fn run(self) -> Result<()> {
        info!("starting symphony service");

        let tracker = Box::new(GitHubIssueTracker::new(
            self.config.repos.clone(),
            self.github_token,
        ));

        let workspace_mgr = WorkspaceManager::new(&self.config.repos)?;

        let agent = Box::new(ClaudeCodeAgent::new(self.config.agent.clone()));

        let mut orchestrator = Orchestrator::new(
            tracker,
            workspace_mgr,
            agent,
            self.config,
        );

        let queue = orchestrator.queue().clone();

        tokio::select! {
            result = orchestrator.run() => result,
            _ = self.shutdown.cancelled() => {
                queue.push(crate::event::SymphonyEvent::Shutdown);
                Ok(())
            }
        }
    }
}
```

**Step 2: Final lib.rs**

```rust
pub mod agent;
pub mod config;
pub mod error;
pub mod event;
pub mod orchestrator;
pub mod queue;
pub mod service;
pub mod tracker;
pub mod workspace;

pub use config::SymphonyConfig;
pub use error::{SymphonyError, Result};
pub use service::SymphonyService;
```

**Step 3: App integration**

Add `rara-symphony.workspace = true` to `crates/app/Cargo.toml` dependencies.

In `crates/app/src/lib.rs`, add `symphony` field to `AppConfig`:

```rust
pub symphony: Option<rara_symphony::SymphonyConfig>,
```

In the `start_with_options` function, after other workers are spawned, add:

```rust
if let Some(ref symphony_config) = config.symphony {
    if symphony_config.enabled {
        let symphony = rara_symphony::SymphonyService::new(
            symphony_config.clone(),
            shutdown_token.clone(),
            std::env::var("GITHUB_TOKEN").ok(),
        );
        tokio::spawn(async move {
            if let Err(e) = symphony.run().await {
                tracing::error!(error = %e, "symphony service failed");
            }
        });
        tracing::info!("symphony service started");
    }
}
```

**Step 4: Verify full build**

Run: `cargo check -p rara-symphony && cargo check -p rara-app`

**Step 5: Commit**

```
feat(symphony): add SymphonyService and integrate with rara-app (#N)
```

---

### Task 9: End-to-End Smoke Test

**Files:**
- Modify: `crates/symphony/src/orchestrator.rs` (add test)

**Step 1: Add integration test with mock tracker**

Add to `orchestrator.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::*;
    use crate::event::IssueState;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    struct MockTracker {
        issues: Vec<TrackedIssue>,
        called: Arc<AtomicBool>,
    }

    #[async_trait::async_trait]
    impl IssueTracker for MockTracker {
        async fn fetch_active_issues(&self) -> Result<Vec<TrackedIssue>> {
            self.called.store(true, Ordering::Relaxed);
            Ok(self.issues.clone())
        }

        async fn fetch_issue_state(&self, _repo: &str, _number: u64) -> Result<IssueState> {
            Ok(IssueState::Active)
        }
    }

    fn test_config() -> SymphonyConfig {
        SymphonyConfig::builder()
            .enabled(true)
            .poll_interval(Duration::from_millis(100))
            .max_concurrent_agents(2)
            .stall_timeout(Duration::from_secs(60))
            .max_retry_backoff(Duration::from_secs(10))
            .workflow_file("WORKFLOW.md")
            .agent(AgentConfig::builder()
                .command("echo")
                .args(vec!["hello".into()])
                .turn_timeout(Duration::from_secs(60))
                .build())
            .repos(vec![])
            .build()
    }

    #[tokio::test]
    async fn orchestrator_polls_and_shuts_down() {
        let called = Arc::new(AtomicBool::new(false));
        let tracker = Box::new(MockTracker {
            issues: vec![],
            called: called.clone(),
        });

        let workspace_mgr = WorkspaceManager::new(&[]).unwrap();
        let agent = Box::new(crate::agent::ClaudeCodeAgent::new(
            AgentConfig::builder()
                .command("echo")
                .args(vec![])
                .turn_timeout(Duration::from_secs(60))
                .build(),
        ));

        let config = test_config();
        let mut orchestrator = Orchestrator::new(tracker, workspace_mgr, agent, config);
        let queue = orchestrator.queue().clone();

        // Schedule shutdown after first poll
        queue.schedule_after(SymphonyEvent::Shutdown, Duration::from_millis(200));

        orchestrator.run().await.unwrap();

        assert!(called.load(Ordering::Relaxed), "tracker should have been polled");
    }
}
```

**Step 2: Run all tests**

Run: `cargo test -p rara-symphony`
Expected: all tests pass

**Step 3: Commit**

```
test(symphony): add orchestrator smoke test with mock tracker (#N)
```

---

## Summary

| Task | Component | Dependencies |
|------|-----------|-------------|
| 1 | Crate scaffolding + errors | None |
| 2 | Config types | Task 1 |
| 3 | EventQueue + events | Task 1 |
| 4 | IssueTracker (GitHub) | Task 1, 3 |
| 5 | WorkspaceManager (git2) | Task 1, 2, 3 |
| 6 | CodingAgent trait | Task 1, 3 |
| 7 | Orchestrator (event loop) | Task 3, 4, 5, 6 |
| 8 | Service + app integration | Task 7 |
| 9 | Smoke test | Task 7 |

**Parallelizable:** Tasks 4, 5, 6 can run in parallel after Tasks 1-3 are done.
