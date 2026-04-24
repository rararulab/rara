# rara-pg-credential-store — Agent Guidelines

## Purpose

SQLite-backed implementation of the `KeyringStore` trait — stores credentials in the `credential_store` table as an alternative to the OS keyring.

## Architecture

### Key module

- `src/lib.rs` — `PgKeyringStore` struct wrapping a `yunara_store::diesel_pool::DieselSqlitePool`. Implements `KeyringStore` with diesel-async DSL queries against the `credential_store` table (columns: `service`, `account`, `value`, `updated_at`). The table schema is defined in `rara-model/src/schema.rs`.

### Note on naming

Despite the "pg" prefix (historical), this implementation uses SQLite, not PostgreSQL. The `credential_store` table is created by migrations in `rara-model`.

## Critical Invariants

- Uses `INSERT ... ON CONFLICT DO UPDATE` for upsert — save is always idempotent.
- The `credential_store` table must exist (created by `rara-model` migrations) before this store is used.
- Uses the `Pg` (diesel query) and `Pool` (bb8 connection-acquire) error variants from `rara-keyring-store`.
- `updated_at` is set via a literal `datetime('now')` SQL fragment — the only sanctioned `sql!` use in this crate, per `docs/guides/db-diesel-migration.md` (diesel has no cross-backend DSL helper for the sqlite-specific form).

## What NOT To Do

- Do NOT use this store in tests that don't have a database — use `DefaultKeyringStore` or a mock.
- Do NOT rename to fix the "pg" misnomer without updating all dependents.

## Dependencies

**Upstream:** `rara-keyring-store` (for `KeyringStore` trait, error types), `rara-model` (schema), `yunara-store` (diesel-async pool), `diesel` + `diesel-async`.

**Downstream:** `rara-app` (selects this as the credential store backend when a database pool is available).
