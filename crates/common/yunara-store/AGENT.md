# yunara-store — Agent Guidelines

## Purpose

Database connection pool and key-value store — wraps SQLx SQLite pool creation with `DatabaseConfig` and provides a `KVStore` for simple string key-value persistence.

## Architecture

### Key modules

- `src/config.rs` — `DatabaseConfig` with `bon::Builder`. `open(database_url)` creates and returns a `DBStore`.
- `src/db.rs` — `DBStore` wraps `SqlitePool`. Provides `pool()` accessor and `kv_store()` to get a `KVStore` instance.
- `src/kv.rs` — `KVStore` — simple string key-value store backed by a `kv_store` SQLite table. Methods: `get`, `set`, `delete`, `list_keys`.
- `src/err.rs` — `snafu`-based error types.

### Public API

- `DatabaseConfig` — connection pool configuration (re-exported).
- `DBStore` — database handle (re-exported).
- `KVStore` — key-value operations (re-exported).

## Critical Invariants

- `DatabaseConfig` does not provide default database URLs — the URL must be passed to `open()`.
- The `kv_store` table must exist before `KVStore` is used — it is created by `rara-model` migrations.
- `DBStore` owns the pool — when dropped, the pool is closed.

## What NOT To Do

- Do NOT put repository implementations or business logic in this crate — it is infrastructure only.
- Do NOT hardcode database URLs — they are resolved from `rara_paths::database_dir()` in `rara-app`.
- Do NOT use `KVStore` for structured data — it is string-only; use proper SQLx models for typed data.

## Dependencies

**Upstream:** `sqlx` (SQLite), `bon`, `serde`.

**Downstream:** `rara-app` (creates `DBStore` at startup), `rara-backend-admin` (uses `KVStore` for settings).
