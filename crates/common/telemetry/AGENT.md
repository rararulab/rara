# common-telemetry — Agent Guidelines

## Purpose

Telemetry utilities — logging initialization, tracing subscriber setup (with optional OTLP export), panic hooks, and tracing context propagation.

## Architecture

### Key modules

- `src/logging.rs` — `init_global_logging()` sets up the tracing subscriber with file rotation, optional OTLP tracing export (HTTP or gRPC), and JSON or text log format. Returns guard objects that must be held for the lifetime of the program.
- `src/panic_hook.rs` — Custom panic hook that logs panics via tracing before the default handler.
- `src/tracing_context.rs` — Utilities for propagating tracing context across async boundaries.
- `src/tracing_sampler.rs` — Custom tracing sampler for controlling trace sampling rates.
- `src/profiling.rs` — `init_pyroscope()` wires the Grafana Pyroscope agent (pprof-rs backend) for continuous CPU profiling. Returns a `ProfilingGuard` whose `Drop` performs graceful shutdown (stop → flush → join). Section omitted from YAML = zero overhead, no thread spawned.

### Features

- `tokio-console` — Enables `console-subscriber` for tokio-console debugging. Requires `tokio/tracing`.

## Critical Invariants

- `init_global_logging()` must be called once at startup — it installs a global tracing subscriber.
- The returned guard objects (`Vec<WorkerGuard>`) must be kept alive — dropping them flushes and closes log writers.
- OTLP endpoint configuration comes from `LoggingOptions` — do not hardcode endpoints.
- The `ProfilingGuard` returned from `init_pyroscope()` must be held for the lifetime of the process — dropping it stops the agent and flushes the last batch.
- Pyroscope tags are **process-level only** (`env`, `host`, `build_commit`). Per-request labels (`session_id`, `skill_name`, `user_id`, …) MUST NOT be attached — they explode label cardinality on the server and turn flamegraph queries into table scans.

## What NOT To Do

- Do NOT call `init_global_logging()` more than once — the global subscriber can only be set once.
- Do NOT drop the logging guards early — log output will stop.
- Do NOT import `tracing_subscriber::fmt::init()` directly — use this crate's initialization.
- Do NOT expect Pyroscope to capture async `.await` stalls or tokio mutex contention — `pprof-rs` is OS-thread CPU sampling. For async stall / lock contention diagnosis, enable the `tokio-console` feature flag (separate chore) and connect with the `tokio-console` CLI.
- Do NOT add per-request labels to Pyroscope tags — see Critical Invariants. Process-level tags only.

## Dependencies

**Upstream:** `tracing`, `tracing-subscriber`, `tracing-appender`, `opentelemetry` (optional OTLP), `pyroscope` + `pyroscope_pprofrs` (continuous profiling).

**Downstream:** `crates/cmd` (initializes logging at startup).
