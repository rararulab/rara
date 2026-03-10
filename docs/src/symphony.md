# Symphony

Symphony 是 rara 内置的 issue runner。它轮询 Linear 或 GitHub issue，给每个 issue 创建独立 git worktree，在该 worktree 里启动一次 `ralph run`，并把 issue 状态同步到 tracker。

当前实现不是 `ralph web` / task API 架构。主流程是：

1. 拉取活跃 issue
2. 为 issue 创建或复用 worktree
3. 生成 `PROMPT.md`
4. `spawn ralph run --autonomous`
5. 启动后把 issue 转到 `In Progress`
6. 成功退出后把 issue 转到 `Verify`
7. 失败时保留 workspace 供人工排查

## Runtime Model

```text
Issue Tracker
    |
    v
Symphony poll loop
    |
    +-- WorkspaceManager.ensure_worktree()
    |
    +-- RalphAgent.start()
    |      writes PROMPT.md
    |      spawns `ralph run --autonomous`
    |
    +-- stream stdout/stderr to per-issue log file
    |
    +-- transition issue -> In Progress
    |
    +-- wait for child exit
           |
           +-- success -> transition issue -> Verify
           |
           +-- failure -> keep workspace + log summary
```

`ralph run` 是受管子进程，不会阻塞整个 symphony 服务。Symphony 只在 poll 周期里用非阻塞方式检查子进程状态。

## Issue States

Linear 的默认拉取状态是 `Todo`，终止状态是 `Done`、`Closed`、`Cancelled`、`Canceled`、`Duplicate`。

Symphony 对状态的约定是：

- issue 被拾取并成功启动 `ralph run` 后，转到 `In Progress`
- `ralph run` 成功退出后，转到 `Verify`
- 不会自动转到 `Done`

这意味着：

- `Verify` 之后的人工验证、PR merge、最终关闭 issue 目前不由 symphony 自动处理
- 如果 `ralph run` 启动失败，issue 不会被自动推进到 `In Progress`

## Worktrees

每个 issue 使用独立 branch 和 worktree。默认目录：

```text
~/.config/rara/ralpha/worktress/<repo>/worktrees/<branch>
```

branch 名由 issue number 和 title slug 组成，例如：

```text
issue-42-fix-startup
```

如果同名 branch 已经被别的 worktree checkout，git 会拒绝再次创建，这通常意味着旧的 issue workspace 还在。

## Logs

每个 issue 的 `ralph` 输出都会写入独立日志文件：

```text
~/.config/rara/ralpha/logs/<repo>/<ISSUE_IDENTIFIER>.log
```

例如：

```text
~/.config/rara/ralpha/logs/rararulab/rara/RAR-123.log
```

日志文件包含：

- 一行 `meta` 头，记录 issue、repo、branch、workspace
- 追加的 `stdout`
- 追加的 `stderr`

查看日志：

```bash
lnav ~/.config/rara/ralpha/logs/**/*.log
```

Symphony 自己的 stdout 只保留摘要日志，不直接转发 `ralph` 的原始输出。

## Prompt Contract

Symphony 会在 issue worktree 中写入 `PROMPT.md`，然后在该目录里运行 `ralph run`。

默认 prompt 要求 `ralph`：

- 完成 issue 对应代码修改
- 运行必要验证
- 用系统里的 `linear` CLI 在 Linear issue 上评论自己的 reasoning / implementation plan
- commit 改动
- push branch
- 创建 GitHub pull request
- 用同一个 `linear` CLI 在 Linear issue 上评论 PR link 和简短实现总结

Symphony 当前不会主动检查 PR 是否创建或是否已 merge，这些属于 agent 交付约束，不是服务端状态机的一部分。

## Configuration

示例：

```yaml
symphony:
  enabled: true
  poll_interval: 30s
  max_concurrent_agents: 2
  workflow_file: WORKFLOW.md

  tracker:
    kind: linear
    api_key: $LINEAR_API_KEY
    team_key: RAR
    active_states: [Todo]
    started_issue_state: In Progress
    completed_issue_state: ToVerify

  agent:
    command: ralph
    backend: codex
    core_config_file: ralph.core.yml

  repos:
    - name: rararulab/rara
      url: https://github.com/rararulab/rara
      repo_path: /path/to/repo
```

说明：

- `repo_path` 是主仓库 checkout
- `workspace_root` 可选；不填时默认落到 `~/.config/rara/ralpha/worktress/<repo>/worktrees`
- 对于只通过 `repo:<owner>/<repo>` label 动态发现、但未显式写在 `symphony.repos` 里的仓库，Symphony 会默认使用 `git@github.com:<owner>/<repo>.git` 作为 clone URL
- `workflow_file` 默认为 `WORKFLOW.md`
- `tracker.started_issue_state` 控制 Ralph 成功启动后 issue 要切到哪个 tracker 状态，默认是 `In Progress`
- `tracker.completed_issue_state` 控制 Ralph 成功完成后 issue 要切到哪个 tracker 状态，默认是 `ToVerify`
- `tracker.active_states` 控制 symphony 会从 tracker 拉哪些状态；默认只拉 `Todo`
- `agent.backend` 控制 symphony 在 issue worktree 里执行 `ralph init --force --backend <backend>` 时使用的 backend
- `agent.core_config_file` 指向仓库根目录里的 Ralph core config；默认是 `ralph.core.yml`
- agent 默认执行 `ralph run --autonomous`
- symphony 会先执行 `ralph init --force --backend <backend> -c ralph.core.yml` 生成 worktree 本地 `ralph.yml`
- init 完成后，symphony 会把 `ralph.core.yml` 的配置物化进生成的 `ralph.yml`
- 随后执行 `ralph run`，只使用这个已经物化好的 worktree 本地 `ralph.yml`
- 运行 symphony 前，需要确保 `codex` CLI 已安装，并通过 `OPENAI_API_KEY` 或 `CODEX_API_KEY` 完成认证

## Source Layout

```text
crates/symphony/src/
├── agent.rs      RalphAgent: prompt rendering + child process spawn
├── config.rs     symphony / tracker / repo config
├── error.rs      snafu error types
├── service.rs    poll loop, process lifecycle, issue transitions, log routing
├── tracker.rs    GitHub and Linear issue tracker implementations
└── workspace.rs  git worktree provisioning and cleanup
```
