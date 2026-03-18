# common-telemetry — Agent Guidelines

## Purpose

Telemetry utilities — logging initialization, tracing subscriber setup (with optional OTLP export), panic hooks, and tracing context propagation.

## Architecture

### Key modules

- `src/logging.rs` — `init_global_logging()` sets up the tracing subscriber with file rotation, optional OTLP tracing export (HTTP or gRPC), and JSON or text log format. Returns guard objects that must be held for the lifetime of the program.
- `src/panic_hook.rs` — Custom panic hook that logs panics via tracing before the default handler.
- `src/tracing_context.rs` — Utilities for propagating tracing context across async boundaries.
- `src/tracing_sampler.rs` — Custom tracing sampler for controlling trace sampling rates.

### Features

- `tokio-console` — Enables `console-subscriber` for tokio-console debugging. Requires `tokio/tracing`.

## Critical Invariants

- `init_global_logging()` must be called once at startup — it installs a global tracing subscriber.
- The returned guard objects (`Vec<WorkerGuard>`) must be kept alive — dropping them flushes and closes log writers.
- OTLP endpoint configuration comes from `LoggingOptions` — do not hardcode endpoints.

## What NOT To Do

- Do NOT call `init_global_logging()` more than once — the global subscriber can only be set once.
- Do NOT drop the logging guards early — log output will stop.
- Do NOT import `tracing_subscriber::fmt::init()` directly — use this crate's initialization.

## Dependencies

**Upstream:** `tracing`, `tracing-subscriber`, `tracing-appender`, `opentelemetry` (optional OTLP).

**Downstream:** `crates/cmd` (initializes logging at startup).
