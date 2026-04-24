# rara-model — Agent Guidelines

## Purpose

Database-layer schema and centralized SQL migrations — owns the diesel `schema.rs` and all migration files that define and evolve the SQLite schema.

## Architecture

### Key modules

- `src/lib.rs` — Crate root.
- `src/schema.rs` — `@generated` by `diesel print-schema`; single source of truth for diesel DSL.
- `migrations/` — Diesel-style migration directories (`YYYYMMDDHHMMSS_<name>/{up.sql,down.sql}`), embedded at compile time by consuming binaries via `diesel_migrations::embed_migrations!`.

### Creating new migrations

```bash
just migrate-add <scope>_<description>   # e.g. just migrate-add chat_add_pinned
```

## Critical Invariants

- **NEVER modify already-applied migrations.** Diesel tracks applied IDs in `__diesel_schema_migrations` — changing an applied migration's SQL leaves deployed databases out of sync. If you need to fix a mistake, create a new migration.
- Migrations run at startup in `rara-app::init_infra()` via `diesel_migrations::embed_migrations!`.
- The database is SQLite, stored at `rara_paths::database_dir()/rara.db`.
- `schema.rs` is regenerated via `diesel print-schema` whenever a migration changes the schema — commit the regenerated file.
- If the local database is corrupted, use `just migrate-reset` to rebuild.

## What NOT To Do

- Do NOT edit an existing migration file after it has been applied — create a new one instead.
- Do NOT put repository implementations or business logic here — this crate is purely schema definitions.
- Do NOT hardcode database URLs or config defaults — the database path is resolved via `rara_paths::database_dir()`.
- Do NOT hand-edit `schema.rs` — regenerate it via `diesel print-schema`.

## Dependencies

**Upstream:** `diesel`, `chrono`, `serde`, `uuid`.

**Downstream:** `rara-app` (runs migrations at startup), `yunara-store` (references table definitions via `diesel::table!`), and every repository/service that uses the shared `DieselSqlitePool`.
