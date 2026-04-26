# yunara-store — Agent Guidelines

## Purpose

Shared diesel-async + bb8 SQLite connection pool and a JSON key-value store
used for runtime settings and miscellaneous persisted state.

## Architecture

### Key modules

- `src/diesel_pool.rs` — `DieselSqlitePool` (single bb8 pool), `DieselSqlitePools` (reader + writer bundle), and `build_sqlite_pools`. The reader pool is sized by `DieselPoolConfig::max_connections`; the writer pool is hard-pinned to `max_size=1` because SQLite serialises writers at the file level (#1843). Both pools set `WAL`, `busy_timeout=5000`, `foreign_keys=ON` pragmas once per physical connection via `custom_setup`, and run a best-effort `ROLLBACK` on every checkout via a `bb8::CustomizeConnection` to scrub leaked transactions.
- `src/config.rs` — `DatabaseConfig` with `bon::Builder`; `open(database_url)` wraps `build_sqlite_pools` and returns a `DBStore`.
- `src/db.rs` — `DBStore` wraps `DieselSqlitePools`; exposes `reader()` (concurrent SELECTs) and `writer()` (single-writer mutations) plus `kv_store()`.
- `src/kv.rs` — `KVStore` backed by the `kv_table` SQLite table (JSON values). Full diesel DSL; `batch_set` runs inside a transaction.
- `src/error.rs` — `snafu` error enum covering pool, diesel, and codec failures.

### Public API

- `DatabaseConfig`, `DBStore`, `KVStore`, `DieselSqlitePool`, `DieselSqlitePools`.

## Critical Invariants

- No hardcoded database URLs — caller supplies the URL to `DatabaseConfig::open()`.
- The `kv_table` schema is owned by `rara-model/migrations` and must exist before `KVStore` is used.
- Pragmas are applied on physical-connection establishment, not on every checkout — `bb8` recycles the same connection without re-setup.
- All mutations (`INSERT`/`UPDATE`/`DELETE`/`transaction`) MUST run on the writer pool. Pure SELECTs run on the reader pool. Routing a write to the reader pool re-introduces the contention #1843 was opened to fix.

## What NOT To Do

- Do NOT put repository implementations or business logic here — infrastructure only.
- Do NOT bypass the diesel DSL with `diesel::sql_query` outside the sanctioned fragments (see `docs/guides/db-diesel-migration.md`).
- Do NOT re-introduce the sqlx pool; `yunara-store` is diesel-only post-#1702.

## Dependencies

**Upstream:** `diesel`, `diesel-async`, `bb8`, `rara-model` (schema), `bon`, `serde`.

**Downstream:** `rara-app` (creates `DBStore` at startup), `rara-backend-admin` (uses `KVStore` for settings + `DieselSqlitePool` for data-feed persistence).
