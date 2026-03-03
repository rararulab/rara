# Tap Memory Async Refactor Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Convert the `rara-memory` tape subsystem to async APIs and add detailed code comments/docs while keeping behavior equivalent.

**Architecture:** Keep the current `tape` module split (`anchors`, `context`, `error`, `service`, `store`) but migrate public operations to `async fn`. Storage stays file-backed, using Tokio async filesystem primitives where practical so callers can await tape operations without blocking the runtime.

**Tech Stack:** Rust, Tokio, serde, serde_json, snafu, strum, derive_more

---

### Task 1: Define the async surface in tests

**Files:**
- Modify: `crates/memory/tests/tap_memory.rs`

**Step 1: Write the failing test**

Convert tape tests to `#[tokio::test]` and call the tape API with `.await`.

**Step 2: Run test to verify it fails**

Run: `cargo test -p rara-memory --test tap_memory`
Expected: FAIL because the tape API is still synchronous.

### Task 2: Implement async tape APIs

**Files:**
- Modify: `crates/memory/src/tape/mod.rs`
- Modify: `crates/memory/src/tape/context.rs`
- Modify: `crates/memory/src/tape/service.rs`
- Modify: `crates/memory/src/tape/store.rs`

**Step 1: Write minimal implementation**

Convert `TapMemory`, `FileTapeStore`, and `TapeService` methods to async and use async-safe internals.

**Step 2: Run tests to verify they pass**

Run: `cargo test -p rara-memory --test tap_memory`
Expected: PASS

### Task 3: Add detailed documentation comments

**Files:**
- Modify: `crates/memory/src/tape/mod.rs`
- Modify: `crates/memory/src/tape/error.rs`
- Modify: `crates/memory/src/tape/context.rs`
- Modify: `crates/memory/src/tape/service.rs`
- Modify: `crates/memory/src/tape/store.rs`

**Step 1: Add docs**

Add module docs plus type and method docs explaining storage semantics, anchor behavior, fork/merge rules, and async expectations.

**Step 2: Re-run tests**

Run: `cargo test -p rara-memory --test tap_memory`
Expected: PASS
