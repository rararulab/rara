# rara-model — Agent Guidelines

## Purpose

Database-layer models and centralized SQL migrations — owns the SQLx `FromRow` store models and all migration files that define and evolve the SQLite schema.

## Architecture

### Key modules

- `src/lib.rs` — Crate root. Currently only contains `#![deny(unsafe_code)]`.
- `migrations/` — SQLx migration files (timestamped `.up.sql` / `.down.sql` pairs). These are compiled into `rara-app` via `sqlx::migrate!("../rara-model/migrations")`.

### Migration naming convention

Files follow the pattern `YYYYMMDDHHMMSS_<scope>_<description>.{up,down}.sql`, e.g.:
- `20260304000000_init.up.sql`
- `20260306000000_knowledge_memory_items.up.sql`
- `20260316035511_execution_traces.up.sql`

### Creating new migrations

```bash
just migrate-add <scope>_<description>   # e.g. just migrate-add chat_add_pinned
```

## Critical Invariants

- **NEVER modify already-applied migrations.** SQLx tracks checksums — any change to an applied migration breaks startup. If you need to fix a mistake, create a new migration.
- Migrations run at startup in `rara-app::init_infra()` via `sqlx::migrate!`.
- The database is SQLite, stored at `rara_paths::database_dir()/rara.db`.
- If the local database is corrupted, use `just migrate-reset` to rebuild.

## What NOT To Do

- Do NOT edit an existing migration file after it has been applied — create a new one instead.
- Do NOT put repository implementations or business logic here — this crate is purely schema definitions and store models.
- Do NOT hardcode database URLs or config defaults — the database path is resolved via `rara_paths::database_dir()`.
- Do NOT add `Default` impls for config structs — config must come from YAML.

## Dependencies

**Upstream:** `sqlx`, `chrono`, `serde`, `uuid`.

**Downstream:** `rara-app` (runs migrations at startup), `yunara-store` (may reference model types).
