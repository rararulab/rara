# common — Agent Guidelines

## Purpose

Workspace group containing foundational infrastructure crates shared across the entire rara project.

## Sub-Crates

| Crate | Purpose |
|-------|---------|
| `base` | Shared utilities: `ArcCow`, environment helpers, process groups, readable sizes, shared strings, UUID helpers |
| `error` | Unified error handling with `StatusCode` mapping (HTTP/Tonic), `ErrorExt` trait, `StackError` |
| `runtime` | Tokio runtime management: named runtimes (background, file_io, network_io), task spawning, global init |
| `telemetry` | Logging, tracing (OpenTelemetry), prometheus metrics, panic hooks |
| `worker` | Background task scheduling: interval/cron/once/notify triggers, pausable workers, manager lifecycle |
| `yunara-store` | Generic KV store interface: `DBStore` (SQL-backed), `KVStore` (config-driven) |
| `tool-macro` | Proc macro for deriving `AgentTool` with `#[tool]` attributes |
| `crawl4ai` | HTTP client for Crawl4AI service — converts web pages to markdown via REST |

## Architecture

Layer-based dependency: `error` and `runtime` form the foundation; `worker`, `telemetry`, and `yunara-store` build on top; `tool-macro` is orthogonal (proc macro).

## Critical Invariants

- `error` crate maps domain errors to HTTP/gRPC status codes — all new error types must implement `ErrorExt` for consistent API responses.
- `runtime` global init (`init_global_runtime`) must be called before spawning tasks — it configures named runtimes that other crates depend on.
- `worker` supports graceful shutdown with timeout — do NOT use `tokio::spawn` for recurring tasks, use the worker framework instead.

## What NOT To Do

- Do NOT put business logic in common crates — they are infrastructure only.
- Do NOT add cross-crate dependencies between common sub-crates unless absolutely necessary — keep the dependency graph shallow.
- Do NOT use `impl Display + impl Error` manually — use `snafu` (see `error` crate).

## Dependencies

- **Downstream**: Nearly every crate in the workspace depends on one or more common crates.
- **Upstream**: None (common crates must not depend on domain/kernel/integration crates).
