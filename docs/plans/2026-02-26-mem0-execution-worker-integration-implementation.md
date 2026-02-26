# mem0 Execution Worker Integration Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Expose mem0 SDK methods from the Python execution worker and consume them from `crates/memory` through a new common execution gRPC client.

**Architecture:** Add thin `mem0.*` capabilities in `rara-py-worker`, add a protocol-only execution gRPC client in `crates/common`, and refactor `crates/memory::mem0_client` into a strict typed wrapper over execution `Invoke` calls.

**Tech Stack:** Python (grpc/FastAPI worker, mem0 SDK), Rust (tonic, prost, serde, snafu), protobuf `execution.v1`

---

### Task 1: Wire protobuf exposure for `execution.v1`

**Files:**
- Modify: `api/build.rs`
- Modify: `api/src/lib.rs`
- Test: `cargo check -p rara-api`

**Step 1:** Add `proto/execution/v1/worker.proto` to `tonic_prost_build::compile_protos`.

**Step 2:** Export generated module as `rara_api::pb::execution::v1`.

**Step 3:** Run `cargo check -p rara-api`.

### Task 2: Add common execution gRPC client (protocol only)

**Files:**
- Modify: `crates/common/worker/src/lib.rs` (or new module exports)
- Create/Modify: `crates/common/worker/src/execution_client.rs`
- Modify: `crates/common/worker/Cargo.toml`
- Test: `crates/common/worker/src/execution_client.rs` unit tests

**Step 1:** Write failing unit tests for JSON/Struct conversion and response envelope decoding.

**Step 2:** Implement `ExecutionWorkerClient` with `connect`, `list_capabilities`, and typed `invoke_json`.

**Step 3:** Add error types for transport/worker/codec/protocol failures.

**Step 4:** Run `cargo test -p common-worker execution_client -- --nocapture` (or equivalent filtered tests).

### Task 3: Add Python mem0 capabilities (thin SDK mapping)

**Files:**
- Create: `execution/workers/rara-py-worker/src/python_worker/capabilities/mem0.py`
- Modify: `execution/workers/rara-py-worker/src/python_worker/app/container.py`
- Modify: `execution/workers/rara-py-worker/tests/test_container.py`
- Create/Modify: `execution/workers/rara-py-worker/tests/test_mem0_capability.py`
- Modify: `execution/workers/rara-py-worker/tests/test_grpc_service.py` (list-capabilities assertion if needed)

**Step 1:** Write failing tests for registration and one mem0 handler (`from_config` + `search` mocked).

**Step 2:** Implement thin handlers with module-level instance and exact SDK method names.

**Step 3:** Register all `mem0.<sdk_method>` capabilities.

**Step 4:** Run targeted `pytest` tests.

### Task 4: Refactor Rust mem0 client to gRPC-backed typed client

**Files:**
- Modify: `crates/memory/Cargo.toml`
- Modify: `crates/memory/src/mem0_client.rs`
- Modify: `crates/memory/src/lib.rs` exports if needed
- Modify: call sites/tests as required (`crates/memory/src/manager.rs`)

**Step 1:** Write failing Rust tests for `Mem0Client` typed request/result decoding against mocked execution client adapter (or conversion-only tests if mocking abstraction is minimal).

**Step 2:** Replace HTTP `reqwest` implementation with execution gRPC client dependency.

**Step 3:** Define strict typed request/response structs for supported mem0 SDK methods used by current memory manager plus `from_config` and other `main.py` methods.

**Step 4:** Preserve current public methods (`add_memories`/`search`/`get`/`delete`) via typed wrappers or rename call sites and update callers.

**Step 5:** Run `cargo test -p rara-memory mem0_client -- --nocapture` (or filtered checks if integration tests require services).

### Task 5: Validate end-to-end compile surfaces

**Files:**
- Workspace compile/test only

**Step 1:** Run targeted Rust checks for touched crates: `rara-api`, `common-worker`, `rara-memory`.

**Step 2:** Run targeted Python worker tests for mem0 capabilities and gRPC capability listing.

**Step 3:** Summarize remaining gaps (e.g., requires installed `mem0` Python package for live tests).

