# rara-mcp

MCP (Model Context Protocol) client library built on top of [rmcp](https://github.com/modelcontextprotocol/rust-sdk).

## Features

- **Stdio transport** -- spawn a child process and communicate over stdin/stdout
- **Streamable HTTP transport** -- connect to an HTTP endpoint with optional OAuth 2.0
- **OAuth persistence** -- tokens stored via keyring or file, with automatic refresh
- **Timeout & logging** -- configurable request timeouts and structured tracing

## Modules

| Module | Description |
|--------|-------------|
| `client` | `RmcpClient` -- the main entry point for connecting to MCP servers |
| `oauth` | OAuth 2.0 token storage, refresh, and persistence (`OAuthPersistor`) |
| `logging_client_handler` | Client-side handler that logs server notifications and forwards elicitation requests |
| `error` | Error types using `snafu` |
| `utils` | Shared helpers (env construction, header building, timeout wrapper) |

## Usage

```rust
use rara_mcp::client::RmcpClient;
use rara_mcp::oauth::OAuthCredentialsStoreMode;
use rmcp::model::ClientInfo;

// Stdio transport
let client = RmcpClient::new_stdio_client(
    "npx".into(),
    vec!["-y".into(), "@anthropic/mcp-server".into()],
    None, &[], None,
).await?;

// -- or -- Streamable HTTP transport
let client = RmcpClient::new_streamable_http_client(
    "my-server", "https://mcp.example.com/sse",
    None, None, None,
    OAuthCredentialsStoreMode::Auto,
).await?;

// Initialize (handshake)
let info = client.initialize(ClientInfo::default(), None, elicitation).await?;

// Use the client
let tools = client.list_tools(None, None).await?;
let result = client.call_tool("echo".into(), Some(json!({"msg": "hi"})), None).await?;
```

## Testing

```bash
cargo test -p rara-mcp
```

31 tests covering:

- **Stdio integration** (7) -- real child-process server via `examples/test_mcp_server`
- **HTTP integration** (5) -- in-process axum server with `StreamableHttpService`
- **Unit tests** (19) -- `meta_string`, `is_no_auth_support`, env/header builders

## Diagnostics

The MCP manager now retains the last startup failure for each server and reports it as
`ConnectionStatus::Error { message }` instead of collapsing the state to disconnected.
The backend-admin MCP API maps that state through to `McpServerStatus::Error`, so the
admin UI can show the actual startup failure that blocked the server.

This remediation was added for `RAR-23`, where Pixiv OAuth automation was blocked by
ghost/mobile/browser MCP servers that looked connected even when their startup or tool
initialization had already failed.

When an automation-backed server still cannot complete an OAuth flow, check these in order:

1. Inspect `/api/v1/mcp/servers` or the admin UI for `error` status and the retained message.
2. Review the per-server log buffer for the startup sequence and any `connection failed` entry.
3. Verify OS/device prerequisites outside the MCP transport:
   - Ghost OS accessibility/screen-capture permissions on macOS
   - Mobile device discovery returning at least one target before calling `mobile_open_url`
   - Browser automation server registration exposing tools instead of an empty tool list
4. Retry after fixing the underlying dependency; a successful restart clears the retained error.
