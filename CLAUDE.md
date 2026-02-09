# CLAUDE.md — Job Automation Platform Development Guide

## Communication
- 用中文与用户交流

## Architecture Rules

### Layer Hierarchy
```
Layer 4 (Entry):      job-cli, job-app
Layer 3 (Interface):  job-server (HTTP + gRPC)
Layer 2 (Domain):     job-domain-{job-source,ai,resume,application,interview,scheduler,notify,analytics}
Layer 1 (Infra):      yunara-store, worker, telemetry, runtime
Layer 0 (Foundation): base, error, paths
```

Dependencies flow **downward only**. Never upward.

### Domain Crate Structure

Each domain crate is **self-contained** — it owns its types, errors, repository trait, repository implementation, service, and migrations:

```
crates/domain/{name}/
  src/
    lib.rs          # Module declarations
    types.rs        # Domain types
    error.rs        # Errors using snafu
    repository.rs   # Repository trait + PostgreSQL implementation
    service.rs      # Business logic service
  migrations/       # SQL migrations owned by this module
    001_init.sql
  Cargo.toml
```

### Key Design Rules

1. **Repository implementations belong in domain crates** — NOT in `yunara-store`. `yunara-store` only provides `DBStore` (PgPool wrapper), KV store, and low-level database primitives. Business-layer repository impls live in their respective domain crate.

2. **Migrations belong in domain crates** — Each domain module owns its DB schema. Migrations are in `crates/domain/{name}/migrations/`.

3. **Error handling uses `snafu`** — All library crates use `#[derive(Snafu)]` for error types. No manual `impl Display + impl Error`. Example:
   ```rust
   use snafu::prelude::*;

   #[derive(Debug, Snafu)]
   pub enum NotifyError {
       #[snafu(display("notification not found: {id}"))]
       NotFound { id: uuid::Uuid },

       #[snafu(display("repository error: {source}"))]
       Repository { source: sqlx::Error },
   }
   ```

4. **No re-exports** — Use full `use crate::` paths. Keep imports explicit.

5. **No mock repositories in tests** — Use `testcontainers` to start real PostgreSQL containers. Tests run against real databases to catch SQL errors, type mismatches, and migration issues.

6. **Domain modules communicate via trait abstraction + outbox events** — No direct cross-domain dependencies (except `scheduler` which can depend on other domain traits).

7. **`job-app` is the composition root** — It wires repositories, services, and workers together via dependency injection.

## Development Workflow

### Git Worktrees for Subagents

Subagents MUST work in isolated git worktrees, NOT directly on `main`:

```bash
# Create worktree for an issue
git worktree add .worktrees/issue-{N}-{short-name} -b issue-{N}-{short-name}

# After work is done, merge back
git checkout main
git merge issue-{N}-{short-name}
```

### Issue Management

When creating GitHub issues:
1. Add label `created-by: claude` to identify bot-created issues
2. Add appropriate category labels (e.g., `domain`, `infra`, `refactor`)
3. When a subagent claims an issue, update it to `in progress` state
4. When work is complete, the issue is closed via commit message `Closes #N`

### Commit Style
- Conventional commits: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`
- Each domain crate has: types, error, service, repository (trait + impl)
- Tests colocated in modules with `#[cfg(test)]`

## Testing Strategy

### Unit Tests
- Colocated in `#[cfg(test)]` modules
- Pure logic tests (state machines, validation, type conversions)

### Integration Tests
- Use `testcontainers` crate for real PostgreSQL
- Test repository implementations against real DB
- Verify migrations apply correctly
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

- Do NOT put repository implementations in `yunara-store`
- Do NOT put domain migrations in `yunara-store/migrations/`
- Do NOT put conversion layers (`convert.rs`) in `yunara-store` — type conversions live in the domain crate that owns those types
- Do NOT use manual `impl Display` + `impl Error` — use `snafu`
- Do NOT use mock repositories in tests — use `testcontainers`
- Do NOT work directly on `main` — use worktrees
- Do NOT create issues without proper labels
