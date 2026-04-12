# rara-backend-admin — Agent Guidelines

## Purpose

Unified HTTP admin routes for all backend subsystems — settings management, model listing, MCP server administration, skills registry, chat/session endpoints, and kernel control routes.

## Architecture

### Key modules

- `src/settings/` — Runtime settings CRUD with MVCC versioning. `SettingsSvc` stores settings in a `settings_version` table (append-only log) where every mutation bumps a global version counter. Provides a `SettingsProvider` trait implementation with change notifications via `watch::Receiver`.
  - `service.rs` — `SettingsSvc` backed directly by `SqlitePool` (no KVStore dependency). Public API: `get()`, `set()`, `delete()`, `batch_update()`, `current_version()`, `snapshot(version)`, `list_versions(limit)`, `rollback_to(version)`.
  - `router.rs` — Axum routes under `/api/v1/settings/`. Version endpoints under `/api/v1/settings/versions/` for listing versions, getting current version, snapshots, and rollback.
- `src/chat/` — Chat and session HTTP endpoints (list sessions, send messages, stream responses).
- `src/kernel/` — Kernel control routes (agent info, execution traces, debug endpoints).
- `src/agents/` — Agent manifest listing and management routes.
- `src/mcp/` — MCP server management routes (list, add, remove, reconnect).
- `src/skills/` — Skills registry HTTP routes.
- `src/state.rs` — `BackendState` that holds shared references to all backend services.
- `src/system_routes.rs` — System-level routes (version, health).
- `src/lib.rs` — Crate root, re-exports all modules.

### Data flow

1. `BackendState::init()` is called by `rara-app` during boot with session index, tape service, and settings.
2. `state.routes()` returns an Axum router with all admin endpoints merged.
3. Routes are mounted into the main HTTP server by `rara-app`.

### Settings MVCC model

- The `settings_version` table is an append-only log. Each row contains a version number, the full settings snapshot (JSON), and a timestamp.
- Writes (`set`, `delete`, `batch_update`) read the current snapshot, apply the mutation, bump the version, and append a new row — all within a single SQLite transaction.
- Rollback is **forward-only**: `rollback_to(v)` reads the snapshot at version `v` and appends it as a new version. History is never rewritten.
- `SettingsSvc` depends only on `SqlitePool` — it does NOT use `KVStore` or any external store abstraction.

## Critical Invariants

- `SettingsSvc` is the single source of truth for runtime-mutable settings (LLM keys, Telegram tokens, etc.).
- Settings changes are broadcast via `tokio::sync::watch` — subscribers get notified of all changes.
- Admin routes should not bypass `SettingsSvc` to read/write settings directly in the database.
- Every settings mutation MUST go through the versioned write path — never insert directly into `settings_version`.

## What NOT To Do

- Do NOT put repository implementations in this crate — it provides HTTP routes, not data access.
- Do NOT hardcode settings values — all mutable config goes through `SettingsSvc`.
- Do NOT duplicate route paths — each subsystem owns its own `/api/v1/<domain>/` namespace.
- Do NOT delete rows from `settings_version` — the table is append-only by design.
- Do NOT bypass `SettingsSvc` to write settings directly to SQLite — this breaks version consistency and watch notifications.

## Dependencies

**Upstream:** `rara-kernel` (for `KernelHandle`, session/tape types), `rara-skills`, `rara-mcp`, `axum`, `sqlx`.

**Downstream:** `rara-app` (mounts routes into the HTTP server).
