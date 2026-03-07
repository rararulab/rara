# Symphony — Autonomous Coding Agent Orchestrator

Symphony is rara's built-in system for autonomously dispatching coding agents to work on issues from **Linear** or **GitHub**. It polls the configured issue tracker for eligible issues, creates isolated git worktrees, and spawns Claude Code (or other CLI agents) as subprocesses to implement the work.

Inspired by [OpenAI Symphony](https://github.com/openai/symphony) — teams manage work on a kanban board, agents pick up tasks and execute them autonomously.

## How It Works

```
Linear / GitHub Issues
        │
        ▼
  ┌─────────────────┐
  │  IssueTracker    │◄── LinearIssueTracker (GraphQL)
  │  (pluggable)     │    GitHubIssueTracker (REST)
  └────────┬────────┘
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

## Quick Start (Linear)

1. Create a [Linear Personal API key](https://linear.app/settings/account/security).
2. Set the environment variable: `export LINEAR_API_KEY=lin_api_...`
3. Add a `symphony` section to your config (see [Linear Configuration](#linear-configuration) below).
4. In your Linear project, add labels with the `repo:` prefix to map issues to repos (e.g. `repo:myorg/myrepo`).
5. Create a `WORKFLOW.md` in your repository root (optional — a default prompt is used if absent).
6. Start rara: `rara server`.
7. Move a Linear issue to "Todo" or "In Progress" — symphony picks it up, creates a worktree, and dispatches an agent.

## Quick Start (GitHub)

1. Add the `symphony` section to your config file (see [GitHub Configuration](#github-configuration) below).
2. Create a `WORKFLOW.md` in your repository root (optional).
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
| `{{issue.number}}` | Issue number (numeric) |
| `{{issue.identifier}}` | Human-readable ID (GitHub: `"42"`, Linear: `"RAR-42"`) |
| `{{issue.title}}` | Issue title |
| `{{issue.body}}` | Issue body/description |
| `{{issue.repo}}` | Target repository name (owner/repo) |
| `{{issue.id}}` | Internal ID (GitHub: `owner/repo#42`, Linear: GraphQL UUID) |
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

Symphony supports two issue tracker backends: **Linear** (recommended) and **GitHub**.

### Linear Configuration

```yaml
symphony:
  enabled: true
  poll_interval: 30s
  max_concurrent_agents: 2
  stall_timeout: 30m
  max_retry_backoff: 1h
  workflow_file: WORKFLOW.md
  tracker:
    kind: linear
    api_key: $LINEAR_API_KEY        # supports $ENV_VAR syntax
    project_slug: my-project        # Linear project slugId
    # endpoint: https://api.linear.app/graphql  # override for self-hosted
    # active_states: [Todo, In Progress]        # default
    # terminal_states: [Done, Cancelled, Canceled, Closed, Duplicate]
    # repo_label_prefix: "repo:"               # default
  agent:
    command: claude
    args: []
    allowed_tools: []
  repos:
    - name: myorg/backend
      url: https://github.com/myorg/backend
      repo_path: /code/backend
      workspace_root: /code/backend/.worktrees
    - name: myorg/frontend
      url: https://github.com/myorg/frontend
      repo_path: /code/frontend
      workspace_root: /code/frontend/.worktrees
```

#### Linear 工作流程

1. **创建 API Key** — 在 [Linear Settings > Security](https://linear.app/settings/account/security) 生成 Personal API key
2. **配置 label 映射** — 在 Linear project 中创建以 `repo:` 为前缀的 label（如 `repo:myorg/backend`）
3. **给 issue 打 label** — 每个 issue 必须有一个 `repo:xxx` label，symphony 据此决定在哪个 repo 的 worktree 中执行
4. **状态驱动** — issue 进入 `Todo` 或 `In Progress` 状态时被 symphony 拉取；进入 `Done`/`Cancelled` 时自动停止

```
Linear Board                          Symphony
┌────────┬────────────┬──────────┐
│ Backlog│   Todo     │In Progress│
│        │            │          │
│        │  RAR-42 ◄──┼──────────┼── symphony 拉取
│        │ repo:myorg │          │   → 创建 worktree
│        │ /backend   │          │   → 启动 agent
│        │            │          │
│        │            │  RAR-43  │── agent 正在工作
│        │            │          │
└────────┴────────────┴──────────┘
                                     agent 完成 → 创建 PR
                                     你在 Linear 上 review → 移到 Done
```

#### Tracker Settings (Linear)

| Key | Default | Description |
|-----|---------|-------------|
| `kind` | — (required) | `linear` |
| `api_key` | — (required) | Linear API key，支持 `$ENV_VAR` 语法 |
| `project_slug` | — (required) | Linear project 的 slugId |
| `endpoint` | `https://api.linear.app/graphql` | GraphQL endpoint（自托管时覆盖） |
| `active_states` | `["Todo", "In Progress"]` | 触发 dispatch 的 issue 状态 |
| `terminal_states` | `["Done", "Closed", "Cancelled", ...]` | 终止状态 |
| `repo_label_prefix` | `"repo:"` | label 前缀，用于 issue → repo 映射 |

#### Linear 优先级映射

Linear 内置优先级直接映射：

| Linear Priority | Symphony Priority | 行为 |
|----------------|-------------------|------|
| Urgent (1) | 1 | 最先被 dispatch |
| High (2) | 2 | |
| Medium (3) | 3 | |
| Low (4) | 4 | |
| No priority (0) | 最低 | 最后被 dispatch |

### GitHub Configuration

```yaml
symphony:
  enabled: true
  poll_interval: 5m
  max_concurrent_agents: 2
  stall_timeout: 30m
  max_retry_backoff: 1h
  workflow_file: WORKFLOW.md
  tracker:
    kind: github
    api_key: $GITHUB_TOKEN           # optional, supports $ENV_VAR
  agent:
    command: claude
    args: []
    allowed_tools: []
  repos:
    - name: myorg/myrepo
      url: https://github.com/myorg/myrepo
      repo_path: /path/to/local/repo
      workspace_root: /path/to/local/repo/.worktrees
      active_labels:
        - symphony:ready
      # max_concurrent_agents: 1
      # workflow_file: custom.md
      # hooks:
      #   after_create: "./scripts/setup-worktree.sh"
      #   before_run: "./scripts/pre-agent.sh"
      #   after_run: "./scripts/post-agent.sh"
      #   before_remove: "./scripts/cleanup-worktree.sh"
```

> **Note:** 如果省略 `tracker` 字段，默认使用 GitHub tracker（向后兼容）。

#### Tracker Settings (GitHub)

| Key | Default | Description |
|-----|---------|-------------|
| `kind` | — | `github` |
| `api_key` | none | GitHub PAT，支持 `$ENV_VAR` 语法 |

### Global Settings

| Key | Default | Description |
|-----|---------|-------------|
| `enabled` | `false` | Whether symphony is active |
| `poll_interval` | — (required) | How often to poll for new issues |
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
| `active_labels` | `["symphony:ready"]` | Labels that mark an issue as ready (GitHub only) |
| `max_concurrent_agents` | inherits global | Per-repo agent limit |
| `workflow_file` | inherits global | Per-repo prompt template override |
| `hooks` | none | Lifecycle hook scripts |

## Multi-Repo Support

Symphony can track multiple repositories simultaneously.

**Linear 多 repo**：在同一个 Linear project 中用 label 区分（`repo:myorg/backend`, `repo:myorg/frontend`）。未打 `repo:` label 的 issue 会被跳过并输出警告。

**GitHub 多 repo**：每个 repo 单独配置 `active_labels`。

Agent slots are managed both globally (`max_concurrent_agents`) and per-repo. An issue is only dispatched when both the global and per-repo limits have available capacity.

```yaml
# Linear 多 repo 示例
symphony:
  max_concurrent_agents: 4
  tracker:
    kind: linear
    api_key: $LINEAR_API_KEY
    project_slug: my-team
  repos:
    - name: myorg/frontend
      repo_path: /code/frontend
      workspace_root: /code/frontend/.worktrees
      max_concurrent_agents: 2
    - name: myorg/backend
      repo_path: /code/backend
      workspace_root: /code/backend/.worktrees
      max_concurrent_agents: 2
# Linear issue 打 label "repo:myorg/frontend" 或 "repo:myorg/backend" 即可路由
```
