# integrations — Agent Guidelines

## Purpose

Workspace group containing adapters for external services and protocols.

## Sub-Crates

| Crate | Purpose |
|-------|---------|
| `keyring-store` | OS keyring abstraction trait (`KeyringStore`) + default implementation (macOS Keychain, Linux Secret Service) |
| `codex-oauth` | OpenAI OAuth (Codex) integration with PKCE flow, token persistence, ephemeral callback server on port 1455 |
| `mcp` | Model Context Protocol client + server lifecycle manager, tool bridge to kernel, OAuth token integration |
| `composio` | Composio action orchestration client (v2 & v3 APIs) with typestate pattern and multi-user routing |

## Architecture

Each integration is a self-contained adapter with a clean trait boundary:

- `keyring-store` defines the `KeyringStore` trait.
- `mcp` bridges external MCP servers into the kernel tool system via `tool_bridge.rs`.
- `composio` uses a typestate pattern (`ComposioApi<V2>` / `ComposioApi<V3>`) with a facade for fallback.
- `codex-oauth` runs an ephemeral HTTP server for the OAuth callback — must clean up previous server before starting a new one.

## Critical Invariants

- `KeyringStore` trait is async — implementations must not block the runtime.
- `codex-oauth` callback server binds to port 1455 — only one instance at a time.
- `mcp` manager owns all server lifecycles — do NOT start/stop MCP servers outside the manager.
- `composio` auth providers (`StaticComposioAuthProvider`, `EnvComposioAuthProvider`) must be set before any API calls.

## What NOT To Do

- Do NOT store credentials in plain text — always use `KeyringStore`.
- Do NOT bypass `McpManager` for MCP server lifecycle — it handles error recovery and logging.
- Do NOT add kernel business logic in integration crates — they are adapters only.
- Do NOT hardcode API endpoints or client IDs — use constants or config.

## Dependencies

- **Downstream**: Kernel and app crates consume these integrations.
- **Upstream**: `keyring-store` has no internal deps. `mcp` depends on `rara-kernel` and `keyring-store`. `codex-oauth` depends on `keyring-store`. `composio` is standalone (reqwest only).
