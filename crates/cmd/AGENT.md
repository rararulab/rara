# rara-cli — Agent Guidelines

## Purpose

Binary entry point for the rara application — provides the `rara` CLI with subcommands for server, chat, gateway, and top (process monitoring).

## Architecture

### Key modules

- `src/main.rs` — `Cli` struct with clap-derived subcommands: `server`, `chat`, `top`, `gateway`. Each subcommand loads `AppConfig`, initializes logging, and delegates to the appropriate crate.
- `src/chat/` — Interactive CLI chat mode using `TerminalAdapter`.
- `src/top/` — `top`-like process monitoring command.
- `src/build_info.rs` — Compile-time version and author metadata.

### Subcommands

| Command | Description |
|---|---|
| `rara server` | Start the full application (HTTP + gRPC + kernel + channels) |
| `rara chat` | Interactive terminal chat session |
| `rara gateway` | Supervisor that spawns/monitors/restarts the agent server |
| `rara top` | Process monitoring dashboard |

### Critical startup sequence

1. `rustls::crypto::ring::default_provider().install_default()` — must happen before any TLS usage.
2. `AppConfig::new()` — loads YAML config.
3. `common_telemetry::logging::init_global_logging()` — initializes tracing subscriber.
4. Delegate to `rara_app::run()` or service-specific entry point.

## Critical Invariants

- `install_default()` for rustls crypto provider must be the first meaningful statement in `main()`.
- Config is loaded before logging initialization so telemetry settings are available.
- The gateway command stays alive after supervisor errors for manual intervention.

## What NOT To Do

- Do NOT add business logic here — this crate is a thin CLI shell; logic belongs in `rara-app` or domain crates.
- Do NOT skip the rustls crypto provider installation — all TLS (reqwest, tonic) will fail without it.
- Do NOT initialize logging before loading config — OTLP settings come from config.

## Dependencies

**Upstream:** `rara-app` (application orchestration), `rara-channels` (terminal adapter, telegram bot builder), `rara-paths`, `common-telemetry`.

**Downstream:** None (this is the final binary).
