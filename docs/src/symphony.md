# Symphony

Symphony 是 rara 内置的 issue runner。它轮询 Linear 或 GitHub issue，给每个 issue 创建独立 git worktree，在该 worktree 里启动一次 `ralph run`，并把 issue 状态同步到 tracker。

当前实现不是 `ralph web` / task API 架构。主流程是：

1. 拉取活跃 issue
2. 为 issue 创建或复用 worktree
3. 生成 `PROMPT.md`
4. `spawn ralph run --no-tui`
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
    |      spawns `ralph run --no-tui`
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

Linear 的默认活跃状态是 `Todo` 和 `In Progress`，终止状态是 `Done`、`Closed`、`Cancelled`、`Canceled`、`Duplicate`。

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
- commit 改动
- push branch
- 创建 GitHub pull request
- 在 Linear issue 中评论 PR link

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

  agent:
    command: ralph
    config_file: config/ralph.yml

  repos:
    - name: rararulab/rara
      url: https://github.com/rararulab/rara
      repo_path: /path/to/repo
```

说明：

- `repo_path` 是主仓库 checkout
- `workspace_root` 可选；不填时默认落到 `~/.config/rara/ralpha/worktress/<repo>/worktrees`
- `workflow_file` 默认为 `WORKFLOW.md`
- agent 默认执行 `ralph run --no-tui`

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
