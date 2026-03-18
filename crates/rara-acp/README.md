# rara-acp

Native [ACP (Agent Client Protocol)](https://github.com/anthropics/agent-client-protocol) client for Rara. Spawns external coding agents (Claude, Codex, Gemini) as subprocesses and communicates over stdin/stdout JSON-RPC.

## What it does

- **Spawn & handshake** — start an agent subprocess, negotiate protocol version and capabilities
- **Session & prompt** — create sessions scoped to a working directory, send user prompts, receive streaming responses
- **Event bridge** — convert ACP session notifications (text chunks, tool calls, plans, file I/O) into `AcpEvent`s delivered via a tokio mpsc channel
- **Permission auto-approval** — automatically approve agent tool-use requests (Rara is the orchestrator, not the gatekeeper)
- **Lifecycle management** — kill and reap child processes on shutdown or error, including handshake failures

## Modules

| Module | Role |
|---|---|
| `connection` | `AcpConnection` — subprocess lifecycle, handshake, session, prompt |
| `delegate` | `RaraDelegate` — implements ACP `Client` trait with backpressure-aware event forwarding |
| `registry` | `AgentRegistry` — maps `AgentKind` to spawn commands for built-in agents |
| `events` | `AcpEvent` enum consumed by the kernel |
| `error` | `AcpError` — snafu-based error types |

## Threading

The upstream `agent-client-protocol` crate is `!Send` (uses `async_trait(?Send)` and `spawn_local`). All ACP operations must run on a `tokio::task::LocalSet`. The companion `AcpDelegateTool` in `rara-app` bridges this constraint by running a dedicated `current_thread` runtime on a blocking thread.
