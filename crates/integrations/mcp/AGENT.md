# rara-mcp — Agent Guidelines

## Purpose

Model Context Protocol (MCP) client integration — manages connections to MCP servers, bridges MCP tools into the kernel's tool system, and provides OAuth support for authenticated MCP servers.

## Architecture

### Key modules

- `src/manager/` — `McpManager` that maintains a registry of MCP server connections. Handles connect/disconnect, reconnection of dead servers, and tool discovery.
- `src/client.rs` — MCP client wrapper for communicating with individual MCP servers.
- `src/tool_bridge.rs` — Bridges MCP tools into `rara-kernel`'s `AgentTool` interface, translating tool definitions and execution between MCP and kernel formats.
- `src/logging_client_handler.rs` — MCP client handler with structured logging.
- `src/oauth.rs` — OAuth token management for authenticated MCP server connections.
- `src/utils.rs` — Shared utilities.

### Data flow

1. `McpManager` is initialized at boot with configured MCP servers.
2. On connect, the manager discovers available tools from each server.
3. `tool_bridge` wraps MCP tools as `DynamicToolProvider` for the kernel.
4. When the kernel invokes an MCP tool, the bridge forwards the call to the appropriate MCP server client.
5. `McpManager::reconnect_dead()` is called periodically to restore dropped connections.

## Critical Invariants

- MCP server connections are managed centrally by `McpManager` — do not create standalone clients.
- Tool names from MCP servers may collide with built-in tools — the bridge must handle namespacing.
- Reconnection is idempotent — calling `reconnect_dead()` on healthy servers is a no-op.

## What NOT To Do

- Do NOT bypass `McpManager` to talk to MCP servers directly — it tracks connection state and handles reconnection.
- Do NOT cache MCP tool definitions indefinitely — servers may update their tool lists.

## Dependencies

**Upstream:** `rara-kernel` (for `AgentTool`, `DynamicToolProvider` traits), `async-trait`, `anyhow`.

**Downstream:** `rara-app` (creates `McpManager`, registers as dynamic tool provider).
