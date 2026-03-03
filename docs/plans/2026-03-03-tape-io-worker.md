# Tape IO Worker Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the current tape store's runtime-blocking file strategy with a single dedicated I/O worker thread using append-only file primitives.

**Architecture:** Move all tape filesystem access into one worker thread that owns the file cache and executes serialized requests. Keep the public `FileTapeStore` API async by bridging requests through response channels, and use `rustix` positional I/O (`pread`/`pwrite`) inside the worker to preserve append-only semantics without `seek`-driven read/write syscalls.

**Tech Stack:** Rust, Tokio `oneshot`, `std::sync::mpsc`, `rustix`, JSONL append-only storage

---

### Task 1: Lock The Behavioral Contract

**Files:**
- Modify: `crates/memory/tests/tap_memory.rs`

**Step 1: Keep a regression test for concurrent appends**

Use the existing `JoinSet` test to verify concurrent `append` calls still produce sequential IDs and the full persisted entry set.

**Step 2: Run the targeted tape test**

Run: `cargo test -p rara-memory --test tap_memory`
Expected: all tape tests pass before and after the storage rewrite.

### Task 2: Replace Runtime File I/O With A Worker

**Files:**
- Modify: `crates/memory/src/tape/store.rs`

**Step 1: Collapse store state into an I/O worker**

Replace `tape_files` and async mutexes with a single worker abstraction that:
- owns the in-memory `TapeFile` cache map,
- runs on one dedicated OS thread,
- receives commands over a channel,
- replies via per-request response channels.

**Step 2: Rewrite `TapeFile` around positional I/O**

Implement file reads and writes using `rustix::io::pread` and `rustix::io::pwrite`, preserving:
- incremental read caching via `read_offset`,
- append-only JSONL semantics,
- fork/merge/archive/reset behavior,
- monotonic ID assignment.

**Step 3: Keep the public API async**

`FileTapeStore::{new,list_tapes,fork,merge,reset,read,append,archive}` should remain async, but only await response channels rather than invoking Tokio file APIs.

### Task 3: Wire Dependencies And Re-Verify

**Files:**
- Modify: `crates/memory/Cargo.toml`

**Step 1: Add `rustix` to `rara-memory`**

Use the workspace dependency so the crate can call positional I/O helpers directly.

**Step 2: Run verification**

Run:
- `cargo test -p rara-memory --test tap_memory`
- `cargo test -p rara-memory --lib -- --skip mem0_client::tests::mem0_client_can_reach_python_worker_mem0_capability_via_testcontainer`

Expected:
- tape tests pass,
- library tests pass except the known environment-dependent skipped case.
