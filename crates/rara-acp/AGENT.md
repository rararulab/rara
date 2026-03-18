# rara-acp — Agent Guidelines

## Purpose

ACP (Agent Communication Protocol) client crate — enables Rara to spawn, communicate with, and manage external agent processes over the ACP wire protocol.

## Architecture

- `src/error.rs` — snafu-based error enum (`AcpError`) covering process lifecycle and protocol failures.
- `src/lib.rs` — crate root; re-exports `AcpError`.

Future modules (planned):
- `src/client.rs` — `AcpClient` that wraps `agent-client-protocol` SDK types.
- `src/types.rs` — Rara-specific type adapters bridging ACP SDK ↔ kernel types.

## Critical Invariants

- All ACP sessions must be explicitly closed or the child process may leak. The client must track open sessions and clean up on drop.
- Version negotiation happens during handshake. If the remote agent reports an unsupported version, fail fast with `UnsupportedVersion` rather than attempting degraded operation.

## What NOT To Do

- Do NOT embed agent business logic here — this crate is a transport/protocol client only. Orchestration belongs in `rara-kernel`.
- Do NOT spawn processes without proper cleanup — always pair spawn with a shutdown/kill path.
- Do NOT silence protocol errors — surface them so the kernel can decide retry vs. abort.

## Dependencies

- **Upstream**: `agent-client-protocol` SDK (external), `tokio`, `serde`, `snafu`, `tracing`.
- **Downstream**: consumed by `rara-kernel` (agent subsystem) and potentially `rara-agents`.
