# common-worker — Agent Guidelines

## Purpose

Background worker and task scheduling framework — provides a flexible worker system with multiple trigger types (once, notify, interval, cron), lifecycle hooks, pause/resume control, and graceful shutdown.

## Architecture

### Key modules

- `src/worker.rs` — `Worker` trait (async `work` method + optional `on_start`/`on_shutdown` hooks). `FallibleWorker` and `InfallibleWorker` variants.
- `src/manager.rs` — `Manager` orchestrates worker lifecycle and shared state. Builder API: `manager.worker(impl Worker).name("x").interval(dur).spawn()`.
- `src/context.rs` — `WorkerContext<S>` passed to each work invocation with state, cancellation token, and notify channel.
- `src/trigger.rs` — `Trigger` enum: `Once`, `Notify`, `Interval`, `Cron`, `IntervalOrNotify`, `CronOrNotify`. `PauseMode` controls behavior during pause.
- `src/handle.rs` — Type-safe handles per trigger type: `IntervalHandle`, `CronHandle`, `NotifyHandle`, etc. Traits: `Handle`, `Pausable`, `Notifiable`.
- `src/driver.rs` — Internal execution loop that drives workers according to their trigger.
- `src/builder.rs` — Type-state builder for constructing workers with compile-time trigger validation.
- `src/blocking.rs` — `BlockingWorker` adapter for synchronous work.
- `src/metrics.rs` — Worker execution metrics.

## Critical Invariants

- Workers must be spawned via `Manager` — direct construction bypasses lifecycle management.
- Shutdown is cooperative — workers should check `ctx.cancelled()` and exit promptly.
- The builder enforces trigger selection at compile time — you cannot spawn a worker without configuring a trigger.

## What NOT To Do

- Do NOT spawn workers outside of `Manager` — they won't participate in graceful shutdown.
- Do NOT block inside async `work()` — use `BlockingWorker` for synchronous tasks.
- Do NOT ignore `WorkerContext::cancelled()` — long-running workers will delay shutdown.

## Dependencies

**Upstream:** `tokio`, `async-trait`, `cron` (for cron triggers).

**Downstream:** `rara-kernel` (may use for scheduled jobs), `rara-app` (worker-based background tasks).
