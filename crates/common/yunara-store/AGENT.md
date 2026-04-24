# yunara-store — Agent Guidelines

## Purpose

Shared diesel-async + bb8 SQLite connection pool and a JSON key-value store
used for runtime settings and miscellaneous persisted state.

## Architecture

### Key modules

- `src/diesel_pool.rs` — `DieselSqlitePool` plus `build_sqlite_pool`. The sqlite pool sets `WAL`, `busy_timeout=5000`, `foreign_keys=ON` pragmas once per physical connection via the manager's `custom_setup` hook.
- `src/config.rs` — `DatabaseConfig` with `bon::Builder`; `open(database_url)` wraps `build_sqlite_pool` and returns a `DBStore`.
- `src/db.rs` — `DBStore` wraps `DieselSqlitePool`; provides `pool()` and `kv_store()`.
- `src/kv.rs` — `KVStore` backed by the `kv_table` SQLite table (JSON values). Full diesel DSL; `batch_set` runs inside a transaction.
- `src/error.rs` — `snafu` error enum covering pool, diesel, and codec failures.

### Public API

- `DatabaseConfig`, `DBStore`, `KVStore`, `DieselSqlitePool`.

## Critical Invariants

- No hardcoded database URLs — caller supplies the URL to `DatabaseConfig::open()`.
- The `kv_table` schema is owned by `rara-model/migrations` and must exist before `KVStore` is used.
- Pragmas are applied on physical-connection establishment, not on every checkout — `bb8` recycles the same connection without re-setup.

## What NOT To Do

- Do NOT put repository implementations or business logic here — infrastructure only.
- Do NOT bypass the diesel DSL with `diesel::sql_query` outside the sanctioned fragments (see `docs/guides/db-diesel-migration.md`).
- Do NOT re-introduce the sqlx pool; `yunara-store` is diesel-only post-#1702.

## Dependencies

**Upstream:** `diesel`, `diesel-async`, `bb8`, `rara-model` (schema), `bon`, `serde`.

**Downstream:** `rara-app` (creates `DBStore` at startup), `rara-backend-admin` (uses `KVStore` for settings + `DieselSqlitePool` for data-feed persistence).
