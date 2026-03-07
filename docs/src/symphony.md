# Symphony — Autonomous Coding Agent Orchestrator

Symphony is rara's built-in system for autonomously dispatching coding agents to work on GitHub issues. It polls configured repositories for issues with specific labels, creates isolated git worktrees, and spawns Claude Code (or other CLI agents) as subprocesses to implement the work.

## How It Works

```
GitHub Issues (label: symphony:ready)
        │
        ▼
  ┌─────────────┐
  │  Orchestrator │◄── event loop (tokio::select!)
  │               │
  │  poll_tick    │── fetch issues → IssueDiscovered events
  │  stall_check  │── detect stalled agents → AgentStalled events
  │  event queue  │── process lifecycle events
  └───────┬───────┘
          │
          ▼
  ┌───────────────┐     ┌──────────────────┐
  │ WorkspaceManager│────▶│  git worktree     │
  │ (git2 crate)   │     │  per issue        │
  └───────────────┘     └──────────────────┘
          │
          ▼
  ┌───────────────┐     ┌──────────────────┐
  │  CodingAgent   │────▶│  claude --print   │
  │  (subprocess)  │     │  in worktree dir  │
  └───────────────┘     └──────────────────┘
```

## Quick Start

1. Add the `symphony` section to your config file (see [Configuration](#configuration) below).
2. Create a `WORKFLOW.md` in your repository root (optional — a default prompt is used if absent).
3. Label a GitHub issue with `symphony:ready`.
4. Start rara: `rara server`.
5. Symphony will pick up the issue, create a worktree, and dispatch an agent.

## Issue Lifecycle

Each issue goes through this state machine:

```
 discovered ──▶ queued ──▶ running ──▶ completed
                  │           │
                  │           ├──▶ failed ──▶ retry (with backoff)
                  │           │                  │
                  │           └──▶ stalled        ▼
                  │                          queued (re-dispatch)
                  │
                  └──▶ terminal (issue closed/merged externally)
```

- **discovered**: issue found during poll, matching active labels
- **queued**: waiting for an available agent slot
- **running**: agent subprocess is working in its worktree
- **completed**: agent finished successfully
- **failed**: agent exited with error; retried with exponential backoff
- **stalled**: agent exceeded `stall_timeout` with no progress
- **terminal**: issue was closed/merged outside of symphony

## WORKFLOW.md Template

Symphony uses a `WORKFLOW.md` file as the prompt template for agents. It supports YAML front matter and Handlebars-style template variables.

```markdown
You are working on issue #{{issue.number}}: {{issue.title}}

Repository: {{issue.repo}}

## Description

{{issue.body}}

{% if attempt %}
This is retry attempt {{attempt}}. The previous attempt failed.
Please review what went wrong and try a different approach.
{% endif %}

## Instructions

- Work in the current working directory (the worktree).
- Use conventional commits (feat, fix, refactor, etc.).
- Include issue reference (#{{issue.number}}) in commit messages.
- When finished, create a PR with your changes.
```

Available template variables:

| Variable | Description |
|----------|-------------|
| `{{issue.number}}` | Issue number |
| `{{issue.title}}` | Issue title |
| `{{issue.body}}` | Issue body/description |
| `{{issue.repo}}` | Repository name (owner/repo) |
| `{{issue.id}}` | Full issue ID (owner/repo#number) |
| `{{attempt}}` | Retry attempt number (absent on first try) |

If no `WORKFLOW.md` is found or its body is empty, a built-in default prompt is used.

## Workspace Isolation

Each issue gets its own git worktree:

```
{workspace_root}/
  └── symphony-owner-repo-42/     ← worktree for issue #42
        ├── .git                   (linked to main repo)
        └── (full repo checkout)
```

- Worktrees are created via the `git2` crate
- Branch name is derived from the issue: `symphony-{owner}-{repo}-{number}`
- If a worktree already exists for an issue, it is reused
- Worktrees are cleaned up after the agent completes or the issue reaches terminal state
- Lifecycle hooks (`after_create`, `before_run`, `after_run`, `before_remove`) can run shell scripts at each stage

## Retry & Backoff

When an agent fails:

1. Failure count is incremented for that issue
2. Backoff delay is computed: `min(2^attempt seconds, max_retry_backoff)`
3. A `RetryReady` event is scheduled after the delay
4. The issue is re-dispatched with `attempt` set in the prompt context

| Attempt | Backoff |
|---------|---------|
| 1st     | 2s      |
| 2nd     | 4s      |
| 3rd     | 8s      |
| 4th     | 16s     |
| ...     | capped at `max_retry_backoff` |

## Observability

### Web Dashboard

Navigate to `/symphony` in the rara web UI to see:

- **Stat cards**: running agents, claimed issues, pending retries, tracked repos
- **Running agents table**: issue, repo, title, branch, workspace path, start time
- **Event log**: real-time SSE stream of all symphony events

### REST API

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/symphony/status` | Current snapshot of symphony state |
| `GET` | `/api/symphony/events` | SSE stream of lifecycle events |

#### `GET /api/symphony/status`

```json
{
  "running": [
    {
      "issue_id": "owner/repo#42",
      "repo": "owner/repo",
      "title": "Add widget support",
      "workspace_path": "/path/to/.worktrees/symphony-owner-repo-42",
      "branch": "symphony-owner-repo-42",
      "started_at": "2026-03-07T08:00:00Z"
    }
  ],
  "claimed": ["owner/repo#42"],
  "retries": [],
  "config_summary": {
    "enabled": true,
    "poll_interval_secs": 300,
    "max_concurrent_agents": 2,
    "repos": ["rararulab/rara"]
  },
  "updated_at": "2026-03-07T08:05:00Z"
}
```

#### `GET /api/symphony/events`

Server-Sent Events stream. Each event has a named type (the `kind` field) and JSON data:

```
event: IssueDiscovered
data: {"timestamp":"2026-03-07T08:00:00Z","kind":"IssueDiscovered","issue_id":"owner/repo#42","detail":"discovered issue: Add widget support"}

event: AgentCompleted
data: {"timestamp":"2026-03-07T08:15:00Z","kind":"AgentCompleted","issue_id":"owner/repo#42","detail":"agent completed successfully"}
```

Event kinds: `IssueDiscovered`, `IssueStateChanged`, `AgentCompleted`, `AgentFailed`, `AgentStalled`, `WorkspaceCleaned`, `RetryReady`.

## Configuration

Add an optional `symphony` section to your YAML config:

```yaml
symphony:
  enabled: true
  poll_interval: 5m
  max_concurrent_agents: 2
  stall_timeout: 30m
  max_retry_backoff: 1h
  workflow_file: WORKFLOW.md
  agent:
    command: claude
    args: []
    allowed_tools: []
    # turn_timeout: 10m   # optional per-turn timeout
  repos:
    - name: rararulab/rara
      url: https://github.com/rararulab/rara
      repo_path: /path/to/local/repo
      workspace_root: /path/to/local/repo/.worktrees
      active_labels:
        - symphony:ready
      # max_concurrent_agents: 1   # per-repo override
      # workflow_file: custom.md   # per-repo override
      # hooks:
      #   after_create: "./scripts/setup-worktree.sh"
      #   before_run: "./scripts/pre-agent.sh"
      #   after_run: "./scripts/post-agent.sh"
      #   before_remove: "./scripts/cleanup-worktree.sh"
```

### Global Settings

| Key | Default | Description |
|-----|---------|-------------|
| `enabled` | `false` | Whether symphony is active |
| `poll_interval` | — (required) | How often to poll GitHub for new issues |
| `max_concurrent_agents` | `2` | Max agents running simultaneously across all repos |
| `stall_timeout` | — (required) | Time before an agent is considered stalled |
| `max_retry_backoff` | — (required) | Upper bound on retry backoff duration |
| `workflow_file` | `WORKFLOW.md` | Default prompt template filename |

### Agent Settings

| Key | Default | Description |
|-----|---------|-------------|
| `command` | `claude` | CLI command to invoke the coding agent |
| `args` | `[]` | Additional arguments passed to the agent |
| `allowed_tools` | `[]` | Tools the agent is allowed to use (passed as `--allowedTools`) |
| `turn_timeout` | none | Optional timeout per agent invocation |

### Per-Repo Settings

| Key | Default | Description |
|-----|---------|-------------|
| `name` | — (required) | Repository identifier (owner/repo) |
| `url` | — (required) | Remote URL |
| `repo_path` | — (required) | Local path to the repository checkout |
| `workspace_root` | — (required) | Directory for worktrees |
| `active_labels` | `["symphony:ready"]` | Labels that mark an issue as ready |
| `max_concurrent_agents` | inherits global | Per-repo agent limit |
| `workflow_file` | inherits global | Per-repo prompt template override |
| `hooks` | none | Lifecycle hook scripts |

## Multi-Repo Support

Symphony can track multiple repositories simultaneously. Each repo has its own:

- Workspace root for worktrees
- Active labels filter
- Optional per-repo concurrency limit
- Optional per-repo workflow template
- Optional lifecycle hooks

Agent slots are managed both globally (`max_concurrent_agents`) and per-repo. An issue is only dispatched when both the global and per-repo limits have available capacity.

```yaml
symphony:
  max_concurrent_agents: 4
  repos:
    - name: org/frontend
      repo_path: /code/frontend
      workspace_root: /code/frontend/.worktrees
      max_concurrent_agents: 2
      active_labels: ["symphony:ready", "frontend"]
    - name: org/backend
      repo_path: /code/backend
      workspace_root: /code/backend/.worktrees
      max_concurrent_agents: 2
      active_labels: ["symphony:ready", "backend"]
```
