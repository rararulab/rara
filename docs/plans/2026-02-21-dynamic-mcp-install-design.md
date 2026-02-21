# Dynamic MCP Server Installation via Agent

**Date**: 2026-02-21
**Status**: Approved

## Problem

1. MCP tools are injected into the agent **statically at startup**. `ToolRegistry` is wrapped in `Arc` and frozen — MCP servers added at runtime via `mcp-admin` API don't have their tools available to the agent until restart.
2. Users want to install MCP servers through conversation (e.g., "install this MCP: https://github.com/..."), and the agent should be able to self-serve the installation.

## Design

### Part 1: Dynamic MCP Tool Discovery

**Current flow (static):**
```
Startup → McpToolBridge::from_manager() → tool_registry.register_mcp() → Arc::new(registry) → frozen
```

**New flow (hybrid):**
```
Every prepare_agent_run():
  1. Built-in tools → Arc<ToolRegistry> (unchanged, no lock)
  2. MCP tools → McpToolBridge::from_manager(McpManager) (dynamic, per-request)
  3. Merge both → effective_tools for AgentRunner
```

Key points:
- `ToolRegistry` stays immutable (`Arc<ToolRegistry>`) — only holds built-in tools
- MCP tools are **not** registered into `ToolRegistry` at startup anymore
- `McpToolBridge::from_manager()` is called each time `prepare_agent_run()` runs
- `ManagedClient` already has a 5-minute tool cache (TTL), so this doesn't hit MCP servers on every message
- New MCP servers added via `McpManager.add_server()` are immediately visible on the next conversation turn

### Part 2: Built-in MCP Management Tools

Register these as built-in agent tools (Rust-native, not HTTP):

| Tool | Purpose | McpManager method |
|------|---------|-------------------|
| `install_mcp_server` | Install & start a new MCP server | `add_server(name, config, start=true)` |
| `list_mcp_servers` | List installed servers + status | `list_servers()` + `connected_servers()` |
| `remove_mcp_server` | Uninstall an MCP server | `remove_server(name)` |

These tools call `McpManager` methods directly (same process, no HTTP overhead).

#### `install_mcp_server` schema

```json
{
  "name": "install_mcp_server",
  "description": "Install and start a new MCP server. The agent should read the MCP server's README/docs to determine the correct command, args, and required env vars before calling this tool.",
  "parameters": {
    "server_name": "string (unique identifier, e.g. 'github')",
    "command": "string (e.g. 'npx', 'uvx', 'docker')",
    "args": ["string array (e.g. ['-y', '@modelcontextprotocol/server-github'])"],
    "env": "object (env vars to set, e.g. {\"GITHUB_TOKEN\": \"...\"})",
    "transport": "string? ('stdio' | 'sse', default 'stdio')",
    "url": "string? (required for sse transport)"
  }
}
```

#### `list_mcp_servers` schema

```json
{
  "name": "list_mcp_servers",
  "description": "List all installed MCP servers with their connection status and available tools.",
  "parameters": {}
}
```

#### `remove_mcp_server` schema

```json
{
  "name": "remove_mcp_server",
  "description": "Stop and uninstall an MCP server.",
  "parameters": {
    "server_name": "string"
  }
}
```

### Part 3: Agent Installation Flow

Typical conversation:

```
User: Install this MCP https://github.com/modelcontextprotocol/server-github
Agent: (web fetch → read README → extract install info)
Agent: (call install_mcp_server with extracted config)
Agent: GitHub MCP server installed. It provides N tools: [...].
       Note: GITHUB_PERSONAL_ACCESS_TOKEN is required. Please provide your token.
User: ghp_xxxxx
Agent: (call install_mcp_server again with env var set, or update via remove + re-install)
Agent: Done. GitHub MCP is connected. You can now ask me to work with your repos.
```

The agent uses its existing web fetch capability to read the repo README and uses LLM reasoning to extract the installation command. No structured parser needed.

## Implementation Steps

### Step 1: Dynamic MCP tool discovery
- Modify `ChatService::prepare_agent_run()` to fetch MCP tools from `McpManager` dynamically
- Remove MCP tool registration from `worker_state.rs` startup
- `ToolRegistry` keeps only built-in tools
- Need a way to merge built-in `ToolRegistry` + dynamic MCP tools into a unified tool set for `AgentRunner`

### Step 2: Built-in MCP management tools
- Create new tool implementations in `crates/agents/src/tools/` (or `crates/core/tool-core/`)
- Implement `AgentTool` trait for each (install, list, remove)
- Each tool holds `McpManager` reference
- Register these tools into `ToolRegistry` at startup (they are built-in, static)

### Step 3: Verify end-to-end
- Start rara without any MCP servers
- In chat, ask agent to install an MCP server from a GitHub URL
- Verify: agent reads README, calls install tool, server connects
- Send next message — verify new MCP tools appear in agent's tool list

## Out of Scope

- MCP marketplace / catalog UI (future)
- Auto-detection of needed MCP servers without user request (future)
- `enable/disable/restart` tools (can add later)
- Updating env vars on existing MCP server without reinstall (can add later)
