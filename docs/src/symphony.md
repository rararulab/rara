# Symphony — Issue Tracker ↔ Ralph Task 同步桥梁

Symphony 是 rara 内置的同步服务，负责将 **Linear** 或 **GitHub** 上的 issue 与 ralph 的 task API 双向同步。它轮询 issue tracker 获取活跃 issue，通过 HTTP JSON-RPC 调用 ralph API 创建/查询 task，并根据 task 状态同步回 issue tracker。

Inspired by [OpenAI Symphony](https://github.com/openai/symphony) — teams manage work on a kanban board, ralph picks up tasks and executes them autonomously.

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
  ┌─────────────────┐
  │  IssueSyncer     │◄── determine sync action per issue
  │                  │
  │  Active + no task│── task.create → ralph API
  │  Active + closed │── transition issue → Done
  │  Terminal + run  │── task.cancel → ralph API
  └────────┬────────┘
           ▼
  ┌─────────────────┐     ┌──────────────────┐
  │  RalphClient     │────▶│  ralph-api        │
  │  (HTTP JSON-RPC) │     │  (subprocess)     │
  └─────────────────┘     └──────────────────┘
           ▲
  ┌─────────────────┐
  │ RalphSupervisor  │── spawn, health check, auto-restart
  └─────────────────┘
```

## Quick Start (Linear)

1. Create a [Linear Personal API key](https://linear.app/settings/account/security).
2. Set the environment variable: `export LINEAR_API_KEY=lin_api_...`
3. Add a `symphony` section to your config (see [Linear Configuration](#linear-configuration) below).
4. In your Linear project, add labels with the `repo:` prefix to map issues to repos (e.g. `repo:myorg/myrepo`).
5. Start rara: `rara server`.
6. Move a Linear issue to "Todo" or "In Progress" — symphony syncs it to ralph, which dispatches an agent.

## Quick Start (GitHub)

1. Add the `symphony` section to your config file (see [GitHub Configuration](#github-configuration) below).
2. Label a GitHub issue with `symphony:ready`.
3. Start rara: `rara server`.
4. Symphony syncs the issue to ralph, which dispatches an agent.

## Sync Logic

每个 poll 周期，IssueSyncer 对每个 issue 执行以下决策：

| Issue 状态 | Ralph Task 状态 | 动作 |
|-----------|----------------|------|
| Active | 无 task | `task.create` — 创建 ralph task |
| Active | `open` / `pending` / `running` | 无操作 — task 正在处理 |
| Active | `closed` | 转换 issue → Done |
| Active | `failed` | 无操作 — 等待人工介入 |
| Terminal | `open` / `pending` / `running` | `task.cancel` — 取消 ralph task |
| Terminal | 其他 / 无 task | 无操作 |

## Ralph API Integration

Symphony 通过 HTTP JSON-RPC 与 ralph-api 通信。RalphSupervisor 负责：

- **Spawn**: 启动 `ralph-api --port 13781` 子进程
- **Health check**: 轮询 `/health` 端点等待就绪（最长 30 秒）
- **Auto-restart**: 进程崩溃后自动重启（3 秒延迟）
- **Graceful shutdown**: symphony 停止时终止 ralph-api

### RPC Methods

| Method | Description |
|--------|-------------|
| `task.create` | 创建新 task，可选 `autoExecute` 立即执行 |
| `task.list` | 列出 task，可按 status 过滤 |
| `task.get` | 按 ID 查询单个 task |
| `task.cancel` | 取消运行中或等待中的 task |

## Configuration

Symphony supports two issue tracker backends: **Linear** (recommended) and **GitHub**.

### Linear Configuration

```yaml
symphony:
  enabled: true
  poll_interval: 30s
  tracker:
    kind: linear
    api_key: $LINEAR_API_KEY        # supports $ENV_VAR syntax
    team_key: RAR                   # Linear team key (issue prefix)
    # project_slug: my-project      # optional, filter within team
    # endpoint: https://api.linear.app/graphql  # override for self-hosted
    # active_states: [Todo, In Progress]        # default
    # terminal_states: [Done, Cancelled, Canceled, Closed, Duplicate]
    # repo_label_prefix: "repo:"               # default
  repos:
    - name: myorg/backend
      url: https://github.com/myorg/backend
    - name: myorg/frontend
      url: https://github.com/myorg/frontend
```

#### Linear 工作流程

1. **创建 API Key** — 在 [Linear Settings > Security](https://linear.app/settings/account/security) 生成 Personal API key
2. **配置 label 映射** — 在 Linear project 中创建以 `repo:` 为前缀的 label（如 `repo:myorg/backend`）
3. **给 issue 打 label** — 每个 issue 必须有一个 `repo:xxx` label，symphony 据此决定路由
4. **状态驱动** — issue 进入 `Todo` 或 `In Progress` 状态时被 symphony 拉取并同步到 ralph

```
Linear Board                          Symphony
┌────────┬────────────┬──────────┐
│ Backlog│   Todo     │In Progress│
│        │            │          │
│        │  RAR-42 ◄──┼──────────┼── symphony 拉取
│        │ repo:myorg │          │   → task.create → ralph
│        │ /backend   │          │   → ralph dispatches agent
│        │            │          │
│        │            │  RAR-43  │── ralph agent 正在工作
│        │            │          │
└────────┴────────────┴──────────┘
                                     ralph 完成 → task.closed
                                     symphony 同步 → issue → Done
```

#### Tracker Settings (Linear)

| Key | Default | Description |
|-----|---------|-------------|
| `kind` | — (required) | `linear` |
| `api_key` | — (required) | Linear API key，支持 `$ENV_VAR` 语法 |
| `team_key` | — (required) | Linear team key，如 `RAR`、`ENG`（issue 标识符前缀） |
| `project_slug` | none | 可选，在 team 内按 project 进一步过滤 |
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
  tracker:
    kind: github
    api_key: $GITHUB_TOKEN           # optional, supports $ENV_VAR
  repos:
    - name: myorg/myrepo
      url: https://github.com/myorg/myrepo
      active_labels:
        - symphony:ready
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

### Per-Repo Settings

| Key | Default | Description |
|-----|---------|-------------|
| `name` | — (required) | Repository identifier (owner/repo) |
| `url` | — (required) | Remote URL |
| `active_labels` | `["symphony:ready"]` | Labels that mark an issue as ready (GitHub only) |

## Multi-Repo Support

Symphony can track multiple repositories simultaneously.

**Linear 多 repo**：在同一个 Linear project 中用 label 区分（`repo:myorg/backend`, `repo:myorg/frontend`）。未打 `repo:` label 的 issue 会被跳过并输出警告。

**GitHub 多 repo**：每个 repo 单独配置 `active_labels`。

```yaml
# Linear 多 repo 示例
symphony:
  poll_interval: 30s
  tracker:
    kind: linear
    api_key: $LINEAR_API_KEY
    team_key: RAR
  repos:
    - name: myorg/frontend
      url: https://github.com/myorg/frontend
    - name: myorg/backend
      url: https://github.com/myorg/backend
# Linear issue 打 label "repo:myorg/frontend" 或 "repo:myorg/backend" 即可路由
```

## Architecture

```
crates/symphony/src/
├── client.rs       RalphClient — HTTP JSON-RPC client for ralph API
├── config.rs       SymphonyConfig, TrackerConfig, RepoConfig
├── error.rs        SymphonyError (snafu)
├── lib.rs          Module exports
├── service.rs      SymphonyService — top-level poll loop
├── supervisor.rs   RalphSupervisor — ralph-api process guardian
├── syncer.rs       IssueSyncer — issue ↔ task sync logic
└── tracker.rs      IssueTracker trait + GitHub/Linear implementations
```
