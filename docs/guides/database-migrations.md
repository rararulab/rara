# Database Migrations

> **Migration in progress (#1702)**: rara is migrating from sqlx to diesel. See [db-diesel-migration.md](./db-diesel-migration.md) for strategy and status. Until migration completes, existing `sqlx::migrate!` flow in `rara-app` remains authoritative. Coordinate migration file additions on #1702 to avoid schema drift across the feature branch.

- **Location**: `crates/rara-model/migrations/` (centralized)
- **Never modify already-applied migrations** — SQLx tracks checksums; any change breaks startup
- Schema changes must create a **new migration**, even to fix a previous one
- Use `just migrate-add <scope>_<description>` to create migrations (e.g., `chat_add_pinned`)
- Use `just migrate-reset` to rebuild when the local database is corrupted
- **Do NOT hardcode database defaults in Rust code** — all config is injected via YAML config file (`~/.config/rara/config.yaml`)
