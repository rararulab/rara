# Symphony: Autonomous Coding Agent Orchestrator

Status: Active
Date: 2026-03-07
Updated: 2026-03-08
Inspired by: [openai/symphony](https://github.com/openai/symphony), [ralph-orchestrator](https://github.com/mikeyobrien/ralph-orchestrator)

## Problem

Rara needs to autonomously pick up coding tasks from issue trackers (GitHub Issues, Linear) across multiple repositories, dispatch them to coding agents, and iterate until completion. This turns issue management into a daemon workflow instead of manual supervision.

## Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Task source | GitHub Issues / Linear | Multi-tracker support via `IssueTracker` trait |
| Agent execution | `ralph run` subprocess | Iterative execution with backpressure, hooks, hat system |
| Workspace isolation | Git worktree via `git2` crate | Lightweight, shared .git objects, pure Rust |
| Multi-repo | Yes, per-repo config | We track issues across multiple repos |
| Crate location | `crates/symphony/` | Independent component, spawned by app |
| Monitoring | Ralph web dashboard (sidecar) | Built-in UI for session/event inspection |

## Architecture

```
                        rara-app (spawn)
                             |
                        Symphony Service
                             |
              +--------------+--------------+
              |              |              |
         IssueTracker   Orchestrator   WorkspaceManager
       (GitHub / Linear) (state machine)  (git2 worktree)
              |              |              |
              |         RalphAgent          |
              |    (write PROMPT.md +       |
              |     spawn `ralph run`)      |
              |              |              |
              +--------------+--------------+
                             |
                        Ralph CLI
                   (iterative execution)
                   ├─ backpressure hooks
                   ├─ hat system
                   ├─ memory
                   └─ web dashboard (sidecar)
```

## Components

### 1. IssueTracker

Polls issue trackers for eligible work items. Supports GitHub Issues and Linear.

```rust
#[async_trait]
pub trait IssueTracker: Send + Sync {
    async fn fetch_active_issues(&self) -> Result<Vec<TrackedIssue>>;
    async fn fetch_issue_state(&self, issue_id: &str) -> Result<IssueState>;
    async fn transition_issue(&self, issue: &TrackedIssue, state: &str) -> Result<()>;
}
```

Implementations:
- `GitHubIssueTracker` — GitHub REST API, filters by labels
- `LinearIssueTracker` — Linear GraphQL API, filters by team + project + states

**Candidate selection rules:**
- Issue is open + has active label (e.g. `symphony:ready`)
- Not already running or claimed
- Concurrency slots available (global + per-repo)
- Sorted by: priority → created_at oldest first → number

### 2. WorkspaceManager

Manages per-issue git worktrees using `git2`.

```rust
pub struct WorkspaceManager { /* per-repo configs */ }

impl WorkspaceManager {
    pub fn ensure_worktree(&self, issue: &TrackedIssue) -> Result<WorkspaceInfo>;
    pub fn cleanup_worktree(&self, issue: &TrackedIssue) -> Result<()>;
}
```

**Hooks** (shell scripts, optional per-repo):
- `after_create` — e.g., `cargo fetch`, `npm install`
- `before_run` — pre-agent setup
- `after_run` — post-agent cleanup
- `before_remove` — final cleanup before worktree deletion

### 3. RalphAgent

Writes a `PROMPT.md` to the worktree and spawns `ralph run` as a subprocess.
Ralph handles all iteration, backpressure, and claude interaction internally.

```rust
pub struct RalphAgent {
    config: AgentConfig,
}

impl RalphAgent {
    /// Build prompt from issue context + workflow template.
    pub fn build_prompt(&self, task: &AgentTask) -> Result<String>;

    /// Write PROMPT.md to worktree and spawn `ralph run`.
    pub async fn start(&self, task: &AgentTask, workspace: &WorkspaceInfo) -> Result<AgentHandle>;
}
```

**Prompt flow:**
1. Read `WORKFLOW.md` from worktree (Handlebars template, optional)
2. Render template with issue context (number, title, body, attempt)
3. Write rendered prompt to `PROMPT.md` in worktree
4. Ralph reads `PROMPT.md` and iterates until `LOOP_COMPLETE`

**Ralph exit codes:**

| Exit Code | Meaning | Symphony Action |
|-----------|---------|-----------------|
| 0 | LOOP_COMPLETE (success) | `AgentCompleted` |
| 1 | Consecutive failures / error | `AgentFailed` (retry) |
| 2 | Max iterations / runtime / cost | `AgentFailed` (retry with backoff) |
| 130 | Interrupted (SIGINT) | `AgentFailed` (no retry) |

### 4. Orchestrator (Event Loop)

Event-loop driven state machine using crossbeam lock-free queue.

```rust
pub struct Orchestrator {
    tracker: Box<dyn IssueTracker>,
    workspace_mgr: WorkspaceManager,
    agent: RalphAgent,
    config: SymphonyConfig,
    queue: EventQueue,
    running: HashMap<String, RunState>,
    claimed: HashSet<String>,
    retries: HashMap<String, RetryEntry>,
}
```

**Events:**
```rust
pub enum SymphonyEvent {
    IssueDiscovered { issue: TrackedIssue },
    IssueStateChanged { issue_id: String, new_state: IssueState },
    AgentCompleted { issue_id: String, workspace: WorkspaceInfo },
    AgentFailed { issue_id: String, workspace: WorkspaceInfo, reason: String },
    AgentStalled { issue_id: String },
    WorkspaceCleaned { issue_id: String, path: PathBuf },
    RetryReady { issue_id: String },
    Shutdown,
}
```

**Retry backoff:** `min(10s * 2^(attempt-1), max_retry_backoff)`

### 5. Ralph Web Dashboard (Sidecar)

Symphony optionally spawns `ralph web` as a sidecar process on startup,
providing a web UI for monitoring agent sessions and events.

```yaml
ralph_web:
  enabled: true
  port: 3000
```

### 6. Symphony Service (Entry Point)

```rust
pub struct SymphonyService { /* config, shutdown, status_handle */ }

impl SymphonyService {
    pub async fn run(self) -> Result<()> {
        // 1. Create tracker, workspace manager, agent
        // 2. Spawn ralph web sidecar (if enabled)
        // 3. Run orchestrator event loop
    }
}
```

## Configuration

```yaml
symphony:
  enabled: true
  poll_interval: "30s"
  max_concurrent_agents: 2
  stall_timeout: "30m"
  max_retry_backoff: "1h"
  workflow_file: "WORKFLOW.md"

  # Issue tracker (GitHub or Linear)
  tracker:
    kind: linear
    api_key: "$LINEAR_API_KEY"
    team_key: "RAR"

  # Agent execution via ralph
  agent:
    command: "ralph"
    config_file: config/ralph.yml  # optional
    extra_args: ["--no-tui"]

  # Ralph web dashboard sidecar
  ralph_web:
    enabled: true
    port: 3000

  repos:
    - name: "rararulab/rara"
      url: "https://github.com/rararulab/rara"
      repo_path: "/home/ryan/code/rararulab/rara"
      workspace_root: "/home/ryan/code/rararulab/rara/.worktrees"
      active_labels: ["symphony:ready"]
      hooks:
        after_create: "cargo fetch"
```

Ralph's own configuration (backend, hooks, max_iterations, etc.) lives in
`config/ralph.yml` in the project or specified via `config_file`.

## State Machine

```
            fetch
  [idle] ──────────► [queued]
                        │
                   ensure_worktree
                   + write PROMPT.md
                   + spawn ralph run
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
- Structured logs: issue ID, repo, workspace path, ralph exit code
- Ralph web dashboard: real-time session monitoring, event inspection
- Ralph stdout/stderr streamed to symphony logs

## Out of Scope (Future)

- MCP-based agent interaction
- PR review / CI status monitoring
- Auto-merge on CI green
- Agent-to-agent handoff (e.g., code review agent)
