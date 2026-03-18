# rara-composio — Agent Guidelines

## Purpose

Composio API client — provides a unified facade for listing and executing Composio actions (third-party tool integrations) with automatic v3/v2 API fallback and connected account resolution.

## Architecture

### Key modules

- `src/lib.rs` — `ComposioClient` public facade. Owns both `ComposioApi<V2>` and `ComposioApi<V3>` typestate clients. All public methods try v3 first, fall back to v2.
- `src/auth.rs` — `ComposioAuth`, `ComposioAuthProvider` trait, `StaticComposioAuthProvider` (fixed key), `EnvComposioAuthProvider` (reads from env/settings).
- `src/v2.rs` — Composio v2 REST API implementation (legacy action names, uppercase format).
- `src/v3.rs` — Composio v3 REST API implementation (tool slugs, lowercase-hyphenated format).

### Key behaviors

- **Action name normalization**: v3 uses lowercase-hyphenated slugs (`github-create-issue`), v2 uses uppercase-underscored names (`GITHUB_CREATE_ISSUE`). The client tries both formats.
- **Connected account resolution**: Automatically looks up the connected account for an app/entity pair and caches it in memory.
- **OAuth connection flow**: `get_connection_url()` generates OAuth links for connecting new apps.

## Critical Invariants

- API key and entity ID come from `ComposioAuthProvider` — never hardcode them.
- v3 is always tried first, v2 only on v3 failure — this ensures forward compatibility.
- Connected account cache is per-process and not persisted — cache misses trigger a fresh API call.
- Error messages are sanitized to avoid leaking account IDs.

## What NOT To Do

- Do NOT call v2 API directly — always go through `ComposioClient` which handles fallback.
- Do NOT store API keys in this crate — use `ComposioAuthProvider`.
- Do NOT skip connected account resolution — many actions require it.

## Dependencies

**Upstream:** `reqwest` (HTTP client), `parking_lot` (RwLock for cache), `serde`/`serde_json`.

**Downstream:** `rara-app` (composio tool implementation uses this client).
