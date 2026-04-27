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
- `src/attrs.rs` — **Stable telemetry attribute keys (Layer A contract).** `SCHEMA_VERSION = "0.1.0"`. Re-exports `gen_ai.*` keys from `opentelemetry-semantic-conventions` and adds the `rara.*` namespace + OpenInference `openinference.span.kind`. Renaming any `pub const` here breaks the external detector — that is a major version bump.
- `src/identifiers.rs` — `pub const` for every tool name, agent name, and guard rule name in rara. Internal call sites that emit `tool.name` / `rara.skill.name` / `rara.guard.rule` MUST reference these constants so renames are caught at compile time.
- `src/payload_sampler.rs` — Layer B content sampler. `PayloadSamplingConfig` (no `Default`, defaults come from YAML) + `PayloadSampler::decide(Outcome)` returns a `SamplingDecision` enum. Deterministic, lock-free, RNG-free.

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
- Do NOT hardcode telemetry attribute strings (`"rara.session.id"`, `"gen_ai.request.model"`) at call sites — always reference the `pub const` in `attrs.rs` so the compiler catches schema drift.
- Do NOT rename or remove a `pub const` in `attrs.rs` without bumping `SCHEMA_VERSION` — downstream detectors pin against the schema.
- Do NOT add Layer B payload attributes outside the sampler — Layer A attributes are always-on and low-cardinality; Layer B (raw prompts/completions) MUST go through `PayloadSampler::decide` first.

## Dependencies

**Upstream:** `tracing`, `tracing-subscriber`, `tracing-appender`, `opentelemetry` (optional OTLP), `pyroscope` + `pyroscope_pprofrs` (continuous profiling).

**Downstream:** `crates/cmd` (initializes logging at startup).
