# rara-backend-admin — Agent Guidelines

## Purpose

Unified HTTP admin routes for all backend subsystems — settings management, model listing, MCP server administration, skills registry, chat/session endpoints, and kernel control routes.

## Architecture

### Key modules

- `src/settings/` — Runtime settings CRUD backed by a KV store. `SettingsSvc` loads settings at startup and provides a `SettingsProvider` trait implementation with change notifications via `watch::Receiver`.
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

## Critical Invariants

- `SettingsSvc` is the single source of truth for runtime-mutable settings (LLM keys, Telegram tokens, etc.).
- Settings changes are broadcast via `tokio::sync::watch` — subscribers get notified of all changes.
- Admin routes should not bypass `SettingsSvc` to read/write settings directly in the KV store.

## What NOT To Do

- Do NOT put repository implementations in this crate — it provides HTTP routes, not data access.
- Do NOT hardcode settings values — all mutable config goes through `SettingsSvc`.
- Do NOT duplicate route paths — each subsystem owns its own `/api/v1/<domain>/` namespace.

## Dependencies

**Upstream:** `rara-kernel` (for `KernelHandle`, session/tape types), `rara-skills`, `rara-mcp`, `yunara-store` (KV store), `axum`.

**Downstream:** `rara-app` (mounts routes into the HTTP server).
