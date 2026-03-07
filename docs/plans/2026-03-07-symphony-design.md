# Symphony: Autonomous Coding Agent Orchestrator

Status: Draft
Date: 2026-03-07
Inspired by: [openai/symphony](https://github.com/openai/symphony)

## Problem

Rara needs to autonomously pick up coding tasks from GitHub Issues across multiple repositories, dispatch them to specialized coding agents (Claude Code, Codex), and track execution to completion. This turns issue management into a daemon workflow instead of manual supervision.

## Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Task source | GitHub Issues | rara's workflow already centers on GitHub |
| Agent interaction | Subprocess (`claude -p`) | Simple, reliable, matches Symphony |
| Workspace isolation | Git worktree via `git2` crate | Lightweight, shared .git objects, pure Rust |
| Multi-repo | Yes, per-repo config | We track issues across multiple repos |
| Crate location | `crates/symphony/` | Independent component, spawned by app |
| Agent abstraction | `CodingAgent` trait | Start with Claude Code, extensible |

## Architecture

```
                        rara-app (spawn)
                             |
                        Symphony Service
                             |
              +--------------+--------------+
              |              |              |
         IssueTracker   Orchestrator   WorkspaceManager
         (GitHub API)   (state machine)  (git2 worktree)
              |              |              |
              |         AgentRunner         |
              |      (subprocess exec)      |
              |              |              |
              +--------------+--------------+
                             |
                    CodingAgent trait
                   /                \
          ClaudeCodeAgent      (future: CodexAgent)
```

## Components

### 1. IssueTracker

Polls GitHub Issues for eligible work items.

```rust
#[async_trait]
pub trait IssueTracker: Send + Sync {
    async fn fetch_active_issues(&self) -> Result<Vec<TrackedIssue>>;
    async fn fetch_issue_state(&self, issue_id: &str) -> Result<IssueState>;
}

pub struct GitHubIssueTracker {
    repos: Vec<RepoConfig>,
    client: reqwest::Client,  // GitHub REST API
}

pub struct TrackedIssue {
    pub id: String,            // "rararulab/rara#42"
    pub repo: String,          // "rararulab/rara"
    pub number: u64,
    pub title: String,
    pub body: String,
    pub labels: Vec<String>,
    pub priority: Option<u32>, // derived from labels
    pub state: IssueState,
    pub created_at: DateTime<Utc>,
}

pub enum IssueState {
    Active,    // open + matching label
    Terminal,  // closed
}
```

**Candidate selection rules** (per Symphony spec):
- Issue is open + has active label (e.g. `symphony:ready`)
- Not already running or claimed
- Concurrency slots available (global + per-repo)
- Sorted by: priority (label-derived) → created_at oldest first → number

### 2. WorkspaceManager

Manages per-issue git worktrees using `git2`.

```rust
pub struct WorkspaceManager {
    configs: HashMap<String, RepoWorkspaceConfig>,  // repo_name -> config
}

pub struct RepoWorkspaceConfig {
    pub repo_path: PathBuf,       // local clone/checkout path
    pub workspace_root: PathBuf,  // where worktrees go
}

impl WorkspaceManager {
    /// Create or reuse a worktree for an issue.
    /// Path: <workspace_root>/issue-<N>-<sanitized_title>
    /// Branch: issue-<N>-<sanitized_title>
    pub fn ensure_worktree(&self, issue: &TrackedIssue) -> Result<WorkspaceInfo>;

    /// Remove worktree and branch for a completed/closed issue.
    pub fn cleanup_worktree(&self, issue: &TrackedIssue) -> Result<()>;
}

pub struct WorkspaceInfo {
    pub path: PathBuf,
    pub branch: String,
    pub created_now: bool,  // true if freshly created
}
```

**Lifecycle:**
1. `ensure_worktree()` — creates worktree + branch if not exists, reuses if exists
2. Agent runs in `workspace.path` as cwd
3. `cleanup_worktree()` — `git worktree remove` + `git branch -d` after terminal state

**Hooks** (shell scripts, optional per-repo):
- `after_create` — e.g., `cargo fetch`, `npm install`
- `before_run` — pre-agent setup
- `after_run` — post-agent cleanup
- `before_remove` — final cleanup before worktree deletion

### 3. CodingAgent Trait + ClaudeCodeAgent

```rust
#[async_trait]
pub trait CodingAgent: Send + Sync {
    /// Start agent execution in the given workspace.
    /// Returns a handle to monitor/cancel the running agent.
    async fn start(
        &self,
        task: &AgentTask,
        workspace: &WorkspaceInfo,
    ) -> Result<AgentHandle>;
}

pub struct AgentTask {
    pub issue: TrackedIssue,
    pub prompt: String,          // assembled from template + issue context
    pub workflow_content: String, // WORKFLOW.md content
}

pub struct AgentHandle {
    pub child: tokio::process::Child,
    pub started_at: Instant,
    pub stdout_log: PathBuf,
    pub stderr_log: PathBuf,
}

pub struct ClaudeCodeAgent {
    pub command: String,        // "claude"
    pub extra_args: Vec<String>,
    pub allowed_tools: Vec<String>,
    pub turn_timeout: Duration,
}

impl CodingAgent for ClaudeCodeAgent {
    async fn start(&self, task: &AgentTask, workspace: &WorkspaceInfo) -> Result<AgentHandle> {
        // claude -p "<prompt>" --cwd <workspace.path> --allowedTools ...
        let child = Command::new(&self.command)
            .arg("-p")
            .arg(&task.prompt)
            .current_dir(&workspace.path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        // ...
    }
}
```

**Prompt assembly:**
```
You are working on issue #{number}: {title}

## Issue Description
{body}

## Workflow Guidelines
{workflow_content}

## Instructions
- Work in the current directory (a git worktree)
- Commit your changes with message: "feat(scope): description (#N)"
- Create a PR when done using `gh pr create`
```

### 4. Event System

Symphony 采用 event loop 驱动架构，所有状态变更通过事件传递，与 rara kernel 的设计思路一致。

```rust
pub enum SymphonyEvent {
    // Timer events
    PollTick,                          // periodic issue polling
    StallCheck,                        // periodic stall detection

    // Issue lifecycle
    IssueDiscovered(TrackedIssue),     // new eligible issue found
    IssueStateChanged {                // external state change (e.g. closed)
        issue_id: String,
        new_state: IssueState,
    },

    // Agent lifecycle
    AgentStarted {
        issue_id: String,
        workspace: WorkspaceInfo,
    },
    AgentCompleted {                   // process exit 0
        issue_id: String,
        output: String,
    },
    AgentFailed {                      // process exit != 0
        issue_id: String,
        error: String,
        exit_code: Option<i32>,
    },
    AgentStalled {                     // no activity for stall_timeout
        issue_id: String,
    },

    // Workspace lifecycle
    WorkspaceCreated(WorkspaceInfo),
    WorkspaceCleaned { issue_id: String },

    // Retry
    RetryReady {                       // backoff timer fired
        issue_id: String,
        attempt: u32,
    },

    // System
    Shutdown,
}
```

### 5. Orchestrator (Event Loop)

```rust
pub struct Orchestrator {
    tracker: Arc<dyn IssueTracker>,
    workspace_mgr: WorkspaceManager,
    agent: Arc<dyn CodingAgent>,
    config: SymphonyConfig,

    // Event infrastructure (crossbeam lock-free queue)
    event_queue: Arc<crossbeam_queue::SegQueue<SymphonyEvent>>,
    notify: Arc<tokio::sync::Notify>,  // wake event loop on push

    // Runtime state
    running: HashMap<String, RunState>,   // issue_id -> state
    claimed: HashSet<String>,             // issue_ids being set up
    retries: HashMap<String, RetryEntry>,
}

pub struct RunState {
    pub issue: TrackedIssue,
    pub handle: AgentHandle,
    pub workspace: WorkspaceInfo,
    pub started_at: Instant,
    pub last_activity: Instant,
}
```

**Event loop:**

```rust
impl Orchestrator {
    fn push_event(&self, event: SymphonyEvent) {
        self.event_queue.push(event);
        self.notify.notify_one();
    }

    async fn pop_event(&self) -> SymphonyEvent {
        loop {
            if let Some(event) = self.event_queue.pop() {
                return event;
            }
            self.notify.notified().await;
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        self.push_event(SymphonyEvent::PollTick);

        loop {
            let event = self.pop_event().await;
            match event {
                SymphonyEvent::PollTick => {
                    self.handle_poll_tick().await?;
                    // Schedule next tick
                    self.schedule_after(SymphonyEvent::PollTick, self.config.poll_interval);
                }

                SymphonyEvent::StallCheck => {
                    self.handle_stall_check().await?;
                    self.schedule_after(SymphonyEvent::StallCheck, self.config.stall_timeout / 2);
                }

                SymphonyEvent::IssueDiscovered(issue) => {
                    self.handle_dispatch(issue).await?;
                }

                SymphonyEvent::AgentCompleted { issue_id, output } => {
                    self.handle_agent_completed(&issue_id, &output).await?;
                }

                SymphonyEvent::AgentFailed { issue_id, error, exit_code } => {
                    self.handle_agent_failed(&issue_id, &error, exit_code).await?;
                }

                SymphonyEvent::AgentStalled { issue_id } => {
                    self.handle_agent_stalled(&issue_id).await?;
                }

                SymphonyEvent::IssueStateChanged { issue_id, new_state } => {
                    self.handle_state_changed(&issue_id, new_state).await?;
                }

                SymphonyEvent::RetryReady { issue_id, attempt } => {
                    self.handle_retry(&issue_id, attempt).await?;
                }

                SymphonyEvent::WorkspaceCleaned { issue_id } => {
                    self.running.remove(&issue_id);
                    self.claimed.remove(&issue_id);
                }

                SymphonyEvent::Shutdown => break,

                _ => {} // WorkspaceCreated, AgentStarted logged by handlers
            }
        }
        Ok(())
    }
}
```

**Handler 拆分:**

```rust
impl Orchestrator {
    /// PollTick: fetch issues → emit IssueDiscovered events
    async fn handle_poll_tick(&self) -> Result<()> {
        let issues = self.tracker.fetch_active_issues().await?;
        let eligible = self.filter_and_sort(issues);
        for issue in eligible {
            self.push_event(SymphonyEvent::IssueDiscovered(issue))?;
        }
        Ok(())
    }

    /// IssueDiscovered: claim → worktree → hooks → start agent
    async fn handle_dispatch(&mut self, issue: TrackedIssue) -> Result<()> {
        if !self.has_slots() || self.claimed.contains(&issue.id) {
            return Ok(());
        }
        self.claimed.insert(issue.id.clone());

        let workspace = self.workspace_mgr.ensure_worktree(&issue)?;
        self.run_hook_if_configured("after_create", &workspace, workspace.created_now).await?;
        self.run_hook_if_configured("before_run", &workspace, true).await?;

        let task = self.assemble_task(&issue, &workspace).await?;
        let handle = self.agent.start(&task, &workspace).await?;

        // Spawn a watcher that emits AgentCompleted/AgentFailed on process exit
        // Spawn watcher: pushes event to queue on process exit
        let queue = self.event_queue.clone();
        let notify = self.notify.clone();
        let id = issue.id.clone();
        tokio::spawn(async move {
            let output = handle.child.wait_with_output().await;
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
            notify.notify_one();
        });

        self.running.insert(issue.id.clone(), RunState { issue, handle, workspace, .. });
        Ok(())
    }

    /// AgentFailed: run after_run hook → schedule retry with backoff
    async fn handle_agent_failed(&mut self, issue_id: &str, error: &str, exit_code: Option<i32>) -> Result<()> {
        if let Some(state) = self.running.get(issue_id) {
            self.run_hook_if_configured("after_run", &state.workspace, true).await?;
        }
        let attempt = self.retries.get(issue_id).map_or(1, |r| r.attempt + 1);
        let delay = self.compute_backoff(attempt);
        self.schedule_after(SymphonyEvent::RetryReady {
            issue_id: issue_id.to_string(),
            attempt,
        }, delay);
        Ok(())
    }

    /// AgentCompleted: run after_run hook → cleanup worktree
    async fn handle_agent_completed(&mut self, issue_id: &str, output: &str) -> Result<()> {
        if let Some(state) = self.running.get(issue_id) {
            self.run_hook_if_configured("after_run", &state.workspace, true).await?;
            self.workspace_mgr.cleanup_worktree(&state.issue)?;
        }
        self.push_event(SymphonyEvent::WorkspaceCleaned { issue_id: issue_id.to_string() })?;
        Ok(())
    }

    /// Delayed event scheduling via tokio::spawn + sleep
    fn schedule_after(&self, event: SymphonyEvent, delay: Duration) {
        let queue = self.event_queue.clone();
        let notify = self.notify.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            queue.push(event);
            notify.notify_one();
        });
    }
}
```

**Retry backoff:**
- Clean exit (continuation): 1s delay
- Failure: `min(10s * 2^(attempt-1), max_retry_backoff)`

### 6. Symphony Service (Entry Point)

```rust
pub struct SymphonyService {
    config: SymphonyConfig,
    shutdown: CancellationToken,
}

impl SymphonyService {
    pub fn new(config: SymphonyConfig, shutdown: CancellationToken) -> Result<Self>;

    /// Long-running event loop, spawned by rara-app.
    pub async fn run(self) -> Result<()> {
        let mut orchestrator = Orchestrator::new(
            self.config,
            self.shutdown.clone(),
        )?;

        tokio::select! {
            result = orchestrator.run() => result,
            _ = self.shutdown.cancelled() => {
                orchestrator.push_event(SymphonyEvent::Shutdown);
                Ok(())
            }
        }
    }
}
```

**App integration** (`crates/app/`):
```rust
if config.symphony.enabled {
    let symphony = SymphonyService::new(config.symphony, shutdown_token.clone())?;
    tokio::spawn(async move { symphony.run().await });
}
```

## Configuration

```yaml
symphony:
  enabled: true
  poll_interval: "30s"
  max_concurrent_agents: 5
  stall_timeout: "5m"
  max_retry_backoff: "5m"
  workflow_file: "WORKFLOW.md"  # default, per-repo overridable

  agent:
    command: "claude"
    args: ["-p"]
    turn_timeout: "1h"

  repos:
    - name: "rara"
      url: "https://github.com/rararulab/rara"
      repo_path: "/home/ryan/code/rararulab/rara"
      workspace_root: "/home/ryan/workspaces/rara"
      active_labels: ["symphony:ready"]
      max_concurrent_agents: 3
      workflow_file: "WORKFLOW.md"  # override
      hooks:
        after_create: "cargo fetch"
    - name: "web-app"
      url: "https://github.com/rararulab/web-app"
      repo_path: "/home/ryan/code/rararulab/web-app"
      workspace_root: "/home/ryan/workspaces/web-app"
      active_labels: ["symphony:ready"]
      hooks:
        after_create: "npm install"
```

All duration fields use `humantime-serde` (consistent with existing rara config).

## Crate Dependencies

```toml
[dependencies]
git2 = "0.19"              # worktree management
reqwest = { version = "0.12", features = ["json"] }  # GitHub API
tokio = { version = "1", features = ["process", "time", "sync"] }
crossbeam-queue = "0.3"    # lock-free SegQueue
serde = { version = "1", features = ["derive"] }
humantime-serde = "1"
snafu = "0.8"
tracing = "0.1"
bon = "3"                  # builder pattern
tokio-util = "0.7"         # CancellationToken

# workspace crates
rara-base = { path = "../common/base" }
rara-error = { path = "../common/error" }
rara-telemetry = { path = "../common/telemetry" }
```

## State Machine

```
            fetch
  [idle] ──────────► [queued]
                        │
                   ensure_worktree
                   + start agent
                        │
                        ▼
                    [running] ◄─── retry (backoff)
                    /   |   \
              exit 0  exit!=0  stall/closed
                /       |         \
               ▼        ▼          ▼
         [completed] [failed]  [cancelled]
              │        │          │
              └────────┴──────────┘
                       │
                  cleanup_worktree
                  (if terminal)
```

## Observability

- `tracing` spans per poll tick, per issue dispatch, per agent run
- Structured logs: issue ID, repo, workspace path, agent exit code
- Future: expose metrics via rara's telemetry subsystem

## Out of Scope (Future)

- MCP-based agent interaction (when Claude Code / Codex MCP matures)
- PR review / CI status monitoring
- Auto-merge on CI green
- Web dashboard for Symphony status
- Agent-to-agent handoff (e.g., code review agent)
