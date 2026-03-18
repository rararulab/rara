# common-runtime — Agent Guidelines

## Purpose

Tokio runtime configuration and management — provides pre-configured global runtimes for different workload types (background, file I/O, network I/O) and convenience spawn/block functions.

## Architecture

### Key modules

- `src/global.rs` — Global runtime singletons: `background_runtime()`, `file_io_runtime()`, `network_io_runtime()`. Initialized once via `init_global_runtimes()`. Convenience functions: `spawn_background()`, `spawn_file_io()`, `spawn_network_io()`, `block_on_*()`.
- `src/factory.rs` — `create_current_thread_runtime()` for single-threaded contexts.
- `src/options.rs` — `RuntimeOptions` and `GlobalRuntimeOptions` for configuring thread counts and stack sizes.
- `src/error.rs` — Runtime-specific error types.

## Critical Invariants

- `init_global_runtimes()` must be called before using any global runtime accessor — accessing before init will panic.
- Each workload type has a separate runtime to prevent I/O-heavy tasks from starving compute tasks.
- `block_on_*()` functions block the current thread — never call from within an async context.

## What NOT To Do

- Do NOT create ad-hoc tokio runtimes — use the pre-configured global runtimes.
- Do NOT call `block_on_*()` from async code — it will deadlock.
- Do NOT call `init_global_runtimes()` more than once.

## Dependencies

**Upstream:** `tokio`, `bon` (builder for options).

**Downstream:** `rara-app`, `rara-server`, and other crates that need to spawn tasks on specific runtimes.
