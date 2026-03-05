# CLAUDE.md — Rara Development Guide

## Communication
- 用中文与用户交流

## Project Identity

Rara is a self-evolving, developer-first personal proactive agent built in Rust. It uses a kernel-inspired architecture with heartbeat-driven proactive behavior, 3-layer memory, and a skills system.

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
- **不要在 Rust 代码中硬编码数据库默认值** — 所有配置通过 YAML 配置文件注入（`~/.config/job/config.yaml`）

### Commit Style
- Conventional commits: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`
- Scope matches crate or area: `feat(kernel):`, `fix(web):`, `refactor(memory):`
- Include `(#N)` issue reference in commit message
- Include `Closes #N` in commit body

## What NOT To Do

- Do NOT put repository impls or routes in `yunara-store` — business logic stays in its own crates
- Do NOT use manual `impl Display` + `impl Error` — use `snafu`
- Do NOT use mock repositories in tests — use `testcontainers`
- Do NOT work directly on `main` — always use worktrees for subagent work
- Do NOT create issues without `created-by:claude` label
- Do NOT forget to close issues after merge — `gh issue close` explicitly
- Do NOT leave stale worktrees — clean up after every merge
- Do NOT modify already-applied migration files — create a new migration instead
- Do NOT hardcode database URLs or config defaults in Rust code — use the YAML config file
- Do NOT use noop/hollow trait implementations to糊弄编译器 — trait method 有真正实现时不允许默认空体（silently return `Ok(())` / `Ok(None)` / `vec![]`）；可选 UX hook（`typing_indicator`, lifecycle hooks）是唯一例外
- Do NOT 构造空壳身份对象 — `Principal` 必须通过 `SecuritySubsystem::resolve_principal()` 或 `Principal::from_user()` 从数据库获得完整的 role + permissions，不允许用 placeholder 值存入 Session
