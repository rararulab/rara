# CLAUDE.md — Rara Development Guide

## Communication
- 用中文与用户交流

## Project Identity

Rara is a self-evolving, developer-first personal proactive agent built in Rust. It uses a kernel-inspired architecture with heartbeat-driven proactive behavior, 3-layer memory, and a skills system.

## Architecture

### Layer Hierarchy
```
Layer 5 (Entry):        rara-cmd, rara-app
Layer 4 (Interface):    rara-server (HTTP + gRPC)
Layer 3 (Core):         rara-kernel, rara-boot, rara-channels
Layer 2 (Capabilities): rara-memory, rara-skills, rara-sessions
Layer 1 (Extensions):   rara-git, rara-coding-task, rara-workspace, rara-backend-admin
Layer 0 (Foundation):   base, rara-error, rara-paths, rara-model, yunara-store
```

Cross-cutting integrations: `rara-mcp`, `rara-composio`, `rara-codex-oauth`, `rara-k8s`, `rara-consul`

Dependencies flow **downward only**. Never upward.

### Kernel Architecture

`rara-kernel` is the central orchestrator with 6 core components:

| Component   | Trait              | Purpose                              |
|-------------|--------------------|--------------------------------------|
| LLM         | `LlmApi`           | Chat completion requests             |
| Tool        | `ToolRegistry`     | Tool registration + dispatch         |
| Memory      | `Memory`           | 3-layer memory (State/Knowledge/Learning) |
| Session     | `SessionRepository`| Conversation history persistence     |
| Guard       | `Guard`            | Tool approval + output moderation    |
| Event Bus   | `EventBus`         | Inter-component event broadcasting   |

Key kernel abstractions: `Kernel`, `ProcessTable`, `AgentProcess`, `AgentHandle`, `ProcessHandle`, `EventLoop`, `ApprovalManager`.

### Memory System

3-layer memory via `rara-memory`:

| Service       | Layer     | Role                                   |
|---------------|-----------|----------------------------------------|
| **mem0**      | State     | Structured fact extraction & auto-dedup|
| **Memos**     | Storage   | Human-readable Markdown notes          |
| **Hindsight** | Learning  | 4-network retain / recall / reflect    |

### Skills System

`rara-skills` handles skill discovery, parsing (SKILL.md with YAML frontmatter), installation from GitHub, and prompt generation for LLM injection.

### Channel Adapters

`rara-channels` provides `ChannelAdapter` implementations:
- `TelegramAdapter` — Telegram Bot API via long polling
- `WebAdapter` — WebSocket + SSE for web chat UI
- `TerminalAdapter` — Local terminal interface

### Extensions (Developer Capabilities)

Each extension registers tools/services into the kernel:
- `rara-git` — Git operations
- `rara-coding-task` — Coding task management
- `rara-workspace` — Workspace management
- `rara-backend-admin` — Admin operations
- `rara-k8s` — Kubernetes integration

### Crate Structure Conventions

Domain and capability crates are **self-contained** — own types, errors, repository, service, and routes:

```
crates/{layer}/{name}/
  src/
    lib.rs           # Module declarations
    types.rs         # Domain types
    error.rs         # Errors using snafu, implements IntoResponse
    repository.rs    # Repository trait (if applicable)
    pg_repository.rs # PostgreSQL implementation (if applicable)
    service.rs       # Business logic
    router.rs        # axum routes (if applicable)
  Cargo.toml
```

### Key Design Rules

1. **Repository impls in their own crates** — NOT in `yunara-store`. `yunara-store` only provides `DBStore` (PgPool wrapper) and KV primitives.

2. **Routes in their own crates** — Each crate with HTTP endpoints defines its own `router.rs`. `rara-app` composes them.

3. **Error handling uses `snafu`** — All library crates use `#[derive(Snafu)]`. Errors that surface via HTTP implement `axum::response::IntoResponse` directly.
   ```rust
   #[derive(Debug, Snafu)]
   pub enum KernelError {
       #[snafu(display("session not found: {key}"))]
       NotFound { key: String },
       #[snafu(display("repository error: {source}"))]
       Repository { source: sqlx::Error },
   }
   ```

4. **`rara-app` is the composition root** — Wires kernel, services, channels, routes, and workers via `rara-boot`.

5. **No re-exports** — Use full `use crate::` paths.

6. **No mock repos in tests** — Use `testcontainers` with real PostgreSQL.

### Frontend (`web/`)

- **Stack**: React 19 + Tailwind v4 + shadcn/ui + TanStack Query v5 + React Router v7
- **API client**: `web/src/api/client.ts` (fetch-based), types in `web/src/api/types.ts`
- **Layout**: `DashboardLayout.tsx` with collapsible sidebar
- **Key pages**: Chat, AgentConsole, KernelTop, Skills, McpServers, Settings, CodingTasks
- **Dev server**: Vite proxies `/api` to `localhost:25555`
- **Build check**: `cd web && npm run build`

## Development Workflow

### Issue → Worktree → Subagent → Merge

This is the standard workflow for all feature/refactor work:

```
1. CREATE ISSUE    →  gh issue create + labels
2. CREATE WORKTREE →  git worktree add .worktrees/issue-{N}-{name} -b issue-{N}-{name}
3. DISPATCH        →  Task subagent works in the worktree
4. VERIFY          →  cargo check + npm run build on worktree
5. MERGE           →  git merge issue-{N}-{name} (resolve conflicts if needed)
6. CLEANUP         →  git worktree remove + git branch -d + gh issue close
```

#### Step 1: Create Issue
```bash
gh issue create --title "feat(kernel): event queue sharding" \
  --label "created-by:claude" --label "enhancement" --label "core"
```
Labels: `created-by:claude` (required) + category (`enhancement`, `refactor`, `ui`, `backend`, `core`, `extension`).

#### Step 2: Create Worktree
```bash
git worktree add .worktrees/issue-{N}-{short-name} -b issue-{N}-{short-name}
```

#### Step 3: Dispatch Subagent
- Subagent works **exclusively** in its worktree directory
- Independent issues can be dispatched **in parallel**
- Subagent should commit its work before finishing

#### Step 4: Verify Builds
After subagent completes, verify in the worktree:
```bash
cargo check -p {crate-name}   # Rust backend
cd web && npm run build        # Frontend (if touched)
```

#### Step 5: Merge to Main
```bash
git checkout main
git merge issue-{N}-{short-name}
# If conflicts: resolve → git add → git commit --no-edit
```

#### Step 6: Cleanup
```bash
git worktree remove .worktrees/issue-{N}-{short-name}
git branch -d issue-{N}-{short-name}
gh issue close {N} --comment "Completed in {commit-hash} — {summary}."
```

**Important**: `Closes #N` in commit messages only works when pushed to remote. For local-only workflows, always close issues explicitly with `gh issue close`.

### Parallel Execution

When user requests involve multiple independent changes, split into separate issues and dispatch subagents in parallel:
- Each subagent gets its own worktree and branch
- Merge sequentially to main, resolving conflicts as they arise
- The second merge may need conflict resolution where both branches touched the same files

### Database Migrations

- **Location**: `crates/rara-model/migrations/`（集中式）
- **永远不要修改已应用的迁移** — SQLx 通过 checksum 追踪，任何改动都会破坏启动
- Schema 变更必须创建**新迁移**，即使是修复上一个迁移的错误
- 使用 `just migrate-add <scope>_<description>` 创建迁移（如 `chat_add_pinned`）
- 本地数据库损坏时用 `just migrate-reset` 重建
- **不要在 Rust 代码中硬编码数据库默认值** — 所有配置通过 Consul 或环境变量注入

### Commit Style
- Conventional commits: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`
- Scope matches crate or area: `feat(kernel):`, `fix(web):`, `refactor(memory):`
- Include `(#N)` issue reference in commit message
- Include `Closes #N` in commit body

## Testing

### Unit Tests
- Colocated in `#[cfg(test)]` modules
- Pure logic tests (state machines, validation, type conversions)

### Integration Tests
- Use `testcontainers` for real PostgreSQL
- Pattern:
  ```rust
  #[cfg(test)]
  mod tests {
      use testcontainers::runners::AsyncRunner;
      use testcontainers_modules::postgres::Postgres;

      async fn setup_db() -> PgPool {
          let container = Postgres::default().start().await.unwrap();
          let url = format!("postgres://postgres:postgres@127.0.0.1:{}/postgres",
              container.get_host_port_ipv4(5432).await.unwrap());
          let pool = PgPool::connect(&url).await.unwrap();
          sqlx::migrate!("./migrations").run(&pool).await.unwrap();
          pool
      }
  }
  ```

## What NOT To Do

- Do NOT put repository impls or routes in `yunara-store` — business logic stays in its own crates
- Do NOT use manual `impl Display` + `impl Error` — use `snafu`
- Do NOT use mock repositories in tests — use `testcontainers`
- Do NOT work directly on `main` — always use worktrees for subagent work
- Do NOT create issues without `created-by:claude` label
- Do NOT forget to close issues after merge — `gh issue close` explicitly
- Do NOT leave stale worktrees — clean up after every merge
- Do NOT modify already-applied migration files — create a new migration instead
- Do NOT hardcode database URLs or config defaults in Rust code — use Consul/env vars
