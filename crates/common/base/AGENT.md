# base — Agent Guidelines

## Purpose

Base utilities and common types shared across the workspace — provides foundational building blocks that do not depend on any domain-specific crates.

## Architecture

### Key modules

- `src/readable_size.rs` — `ReadableSize` type for human-friendly byte sizes (e.g. "100 MB"). Used in server configs for body limits and message sizes.
- `src/id.rs` — Common ID type utilities.
- `src/env.rs` — Environment variable helpers.
- `src/process_group.rs` — Process group management utilities for child process lifecycle.
- `src/arc_cow.rs` — `ArcCow` — a `Cow`-like type backed by `Arc` for zero-copy shared ownership.
- `src/shared_string.rs` — `SharedString` — interned/shared string type for reducing allocations.

## Critical Invariants

- This crate must have zero domain dependencies — it sits at the bottom of the dependency tree.
- `ReadableSize` serializes/deserializes as human-readable strings and supports arithmetic operations.

## What NOT To Do

- Do NOT add domain-specific logic here — this crate is for pure utilities.
- Do NOT depend on any `rara-*` crates from here — it would create circular dependencies.

## Dependencies

**Upstream:** `derive_more` (derives for utility types).

**Downstream:** Nearly every crate in the workspace.
