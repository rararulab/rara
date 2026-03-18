# rara-acp — Agent Guidelines

## Purpose

ACP (Agent Communication Protocol) client crate — enables rara to spawn, communicate with, and manage external agent processes (Claude Code, Codex, Gemini) over the ACP wire protocol.

## Architecture

```
AcpThread (Send+Sync handle, public API)
    ├── spawn() → handshake + create session
    ├── prompt() → send prompt, process events, handle permissions
    ├── authorize_tool_call() → resolve pending permission
    ├── cancel() / shutdown()
    │
    └── AcpConnectionActor (!Send, on dedicated LocalSet)
            ├── AcpConnection (subprocess lifecycle)
            └── RaraDelegate (acp::Client impl)
                ├── request_permission() → PermissionBridge → mpsc → AcpThread
                ├── session_notification() → AcpEvent → mpsc → AcpThread
                └── read/write_text_file() → direct tokio::fs
```

Key modules:
- `thread.rs` — AcpThread handle + connection actor + types
- `connection.rs` — Low-level ACP subprocess lifecycle (!Send)
- `delegate.rs` — acp::Client trait impl (permission forwarding or auto-approve, file I/O)
- `events.rs` — AcpEvent enum + PermissionBridge + PermissionOptionInfo
- `registry.rs` — Built-in agent commands (npx-based ACP adapters)
- `error.rs` — snafu error types

## Critical Invariants

- `AcpConnection` is `!Send` — MUST run on `tokio::task::LocalSet`
- `AcpThread` bridges `!Send` → `Send` via mpsc command/event channels
- `PermissionBridge.reply_tx` MUST be consumed — dropping causes `Cancelled`
- One rara Session may have multiple AcpThreads (different agents or tasks)
- AcpThread session ≠ rara Session — they have independent lifecycles
- All ACP sessions must be explicitly shut down or the child process may leak

## What NOT To Do

- Do NOT auto-approve permissions when a PermissionBridge channel is available — always forward to the caller for user confirmation
- Do NOT store AcpThread references in rara's Session — they are task-scoped, created and destroyed within a single tool execution
- Do NOT call AcpConnection methods directly from Send contexts — go through AcpThread's command channel
- Do NOT block the tokio runtime waiting for AcpThread — use async/await
- Do NOT embed agent business logic here — this crate is a transport/protocol client only
- Do NOT silence protocol errors — surface them so the kernel can decide retry vs. abort

## Dependencies

- **Upstream**: `agent-client-protocol` 0.10.2 (ACP JSON-RPC)
- **Downstream**: `rara-app` (AcpDelegateTool), `rara-kernel` (ApprovalManager, EventQueue)
