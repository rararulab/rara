# Database Migrations

rara uses **diesel** for the database layer — see [db-diesel-migration.md](./db-diesel-migration.md) for the strategy record.

- **Location**: `crates/rara-model/migrations/` (centralized)
- **Layout**: diesel-style subdirectories (`YYYYMMDDHHMMSS_<name>/{up.sql,down.sql}`)
- **Runtime**: embedded into `rara-app` at compile time via `diesel_migrations::embed_migrations!` and applied on startup against `__diesel_schema_migrations`
- **Never modify already-applied migrations** — any change to an applied migration leaves deployed databases out of sync with the embedded checksum
- Schema changes must create a **new migration**, even to fix a previous one
- Use `just migrate-add <scope>_<description>` to scaffold a migration pair (wraps `diesel migration generate`)
- Use `just migrate-reset` to rebuild when the local database is corrupted
- `crates/rara-model/src/schema.rs` is `@generated` by `diesel print-schema` — regenerate and commit it after any migration that changes structure
- **Do NOT hardcode database defaults in Rust code** — all config is injected via YAML config file (`~/.config/rara/config.yaml`)

## First boot after the sqlx → diesel cutover

Databases created under the old sqlx flow tracked migrations in `_sqlx_migrations`. After upgrading to the diesel-based binary, run `just migrate-reset` once to rebuild the local DB under the new `__diesel_schema_migrations` table — diesel does not read the sqlx checksum table.

## CLI tooling

```bash
cargo install diesel_cli --no-default-features --features sqlite,postgres
```

`diesel_cli` is used for `diesel print-schema` (regenerating `schema.rs`) and `diesel migration generate` (scaffolding new migration pairs). It is not a runtime dependency.
