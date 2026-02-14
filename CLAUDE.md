# CLAUDE.md — Job Automation Platform Development Guide

## Communication
- 用中文与用户交流

## Architecture

### Layer Hierarchy
```
Layer 4 (Entry):      rara-cmd, rara-app
Layer 3 (Interface):  rara-server (HTTP + gRPC)
Layer 2 (Domain):     rara-domain-{job,ai,resume,application,interview,scheduler,chat,shared,analytics}
Layer 1 (Infra):      rara-sessions, rara-agents, rara-workers, rara-model, telegram-bot
Layer 0 (Foundation): base, error, paths
```

Dependencies flow **downward only**. Never upward.

### Domain Crate Structure

Each domain crate is **self-contained** — owns types, errors, repository, service, routes, and migrations:

```
crates/domain/{name}/
  src/
    lib.rs          # Module declarations
    types.rs        # Domain types (includes From impls for DB conversion)
    error.rs        # Errors using snafu, implements IntoResponse
    repository.rs   # Repository trait
    pg_repository.rs # PostgreSQL implementation
    service.rs      # Business logic
    router.rs       # axum routes (domain owns its HTTP layer)
  migrations/
  Cargo.toml
```

### Key Design Rules

1. **Repository impls in domain crates** — NOT in `yunara-store`. `yunara-store` only provides `DBStore` (PgPool wrapper) and KV primitives.

2. **Routes in domain crates** — Each domain crate defines its own `router.rs` with axum routes. `rara-app` composes them.

3. **Error handling uses `snafu`** — All library crates use `#[derive(Snafu)]`. Domain errors implement `axum::response::IntoResponse` directly.
   ```rust
   #[derive(Debug, Snafu)]
   pub enum ChatError {
       #[snafu(display("session not found: {key}"))]
       NotFound { key: String },
       #[snafu(display("repository error: {source}"))]
       Repository { source: sqlx::Error },
   }
   ```

4. **`rara-app` is the composition root** — Wires services, routes, and workers via dependency injection.

5. **No re-exports** — Use full `use crate::` paths.

6. **No mock repos in tests** — Use `testcontainers` with real PostgreSQL.

### Frontend (`web/`)

- **Stack**: React 19 + Tailwind v4 + shadcn/ui + TanStack Query v5 + React Router v7
- **API client**: `web/src/api/client.ts` (fetch-based), types in `web/src/api/types.ts`
- **Layout**: `DashboardLayout.tsx` with collapsible sidebar
- **Dev server**: Vite proxies `/api` to `localhost:3000`
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
gh issue create --title "feat(chat): model selector" \
  --label "created-by:claude" --label "enhancement" --label "ui"
```
Labels: `created-by:claude` (required) + category (`enhancement`, `refactor`, `ui`, `backend`, `domain`).

#### Step 2: Create Worktree
```bash
git worktree add .worktrees/issue-{N}-{short-name} -b issue-{N}-{short-name}
```

#### Step 3: Dispatch Subagent
- Subagent works **exclusively** in its worktree directory
- Independent issues can be dispatched **in parallel** (e.g., #116 and #117 ran concurrently)
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

### Commit Style
- Conventional commits: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`
- Scope matches crate or area: `feat(chat):`, `fix(web):`, `refactor(sessions):`
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

- Do NOT put repository impls or routes in `yunara-store` — business logic stays in domain crates
- Do NOT use manual `impl Display` + `impl Error` — use `snafu`
- Do NOT use mock repositories in tests — use `testcontainers`
- Do NOT work directly on `main` — always use worktrees for subagent work
- Do NOT create issues without `created-by:claude` label
- Do NOT forget to close issues after merge — `gh issue close` explicitly
- Do NOT leave stale worktrees — clean up after every merge
