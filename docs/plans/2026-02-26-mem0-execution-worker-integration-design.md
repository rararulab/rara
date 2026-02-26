# mem0 via Execution Worker gRPC Integration Design

## Context

We need to integrate mem0 into the Python worker and consume it from Rust memory code through our execution worker gRPC contract (`execution.v1.ExecutionWorkerService`).

Key constraints confirmed:

- Python worker must expose mem0 **SDK methods**, not re-wrap mem0 REST endpoints.
- The set of methods should be derived from what `mem0/server/main.py` uses.
- Capability names must match mem0 SDK method names exactly (prefixed under `mem0.`), e.g. `mem0.from_config`, `mem0.add`.
- Rust does **not** yet have an execution worker gRPC client implementation.
- The new Rust gRPC client must live in `crates/common` and be protocol-focused (execution contract only).
- `crates/memory/src/mem0_client.rs` must become a typesafe mem0 business client on top of that gRPC client.
- Python worker should not add extra synchronization/concurrency logic (no locking layer).
- mem0 configuration should be done via a separate capability first (`mem0.from_config`) that constructs/replaces the SDK instance.
- Rust mem0 client types should be fully strict/typed (no catch-all JSON fields).

Reference: mem0 server maps REST handlers to mem0 SDK methods in `server/main.py`.

## Goals

1. Expose mem0 SDK capabilities from `rara-py-worker` through execution gRPC `Invoke`.
2. Add a reusable Rust execution worker gRPC client in `crates/common`.
3. Refactor `crates/memory::mem0_client` into a strict typed mem0 client using execution gRPC.
4. Preserve separation of concerns:
   - `common`: execution protocol and transport
   - `memory`: mem0 business semantics and typed contracts

## Non-Goals

- Re-implementing mem0 REST routes in Python worker.
- Adding worker-side concurrency control, locks, or request serialization for mem0.
- Designing a generic schema negotiation system for arbitrary capabilities.
- Supporting mem0 SDK methods not used by `server/main.py` in the first iteration.

## High-Level Architecture

### Python side (`rara-py-worker`)

- Add `python_worker.capabilities.mem0` module.
- Maintain a module-level mem0 SDK instance reference (initially unset).
- Register capability handlers matching mem0 SDK method names used by `main.py`:
  - `mem0.from_config`
  - `mem0.add`
  - `mem0.get_all`
  - `mem0.get`
  - `mem0.search`
  - `mem0.update`
  - `mem0.history`
  - `mem0.delete`
  - `mem0.delete_all`
  - `mem0.reset`
- Each handler:
  - accepts execution payload dict
  - calls corresponding mem0 SDK method directly
  - returns SDK result (with minimal JSON-serializable normalization only if required)

### Rust side (`crates/common` + `crates/memory`)

- Add a reusable execution worker gRPC client in `crates/common`:
  - wraps `execution.v1.ExecutionWorkerService`
  - supports `ListCapabilities`, `Invoke`, optionally `SubmitTask/GetTask`
  - converts between Rust JSON values and protobuf `Struct`
  - maps transport/protocol/worker errors
- Refactor `crates/memory/src/mem0_client.rs`:
  - replace direct HTTP calls to mem0 REST with execution worker gRPC invocations
  - expose strongly typed mem0 methods aligned to mem0 SDK capabilities
  - own all mem0 request/response types and capability names

## Capability Contract Strategy

Capability names are based on mem0 SDK method names exactly:

- `mem0.from_config`
- `mem0.add`
- `mem0.get_all`
- `mem0.get`
- `mem0.search`
- `mem0.update`
- `mem0.history`
- `mem0.delete`
- `mem0.delete_all`
- `mem0.reset`

The execution worker contract remains generic (`capability + Struct payload -> Struct result`).
Typed mem0 request/response contracts are defined in Rust `crates/memory` and mirrored by Python handlers.

## Rust API Design

### `crates/common`: Execution Worker gRPC Client (protocol-only)

Provide a client focused on execution protocol mechanics:

- Connect to execution worker gRPC endpoint
- List capabilities
- Invoke capability with JSON payload and decode typed JSON result
- Surface worker-reported errors distinctly from gRPC transport failures

Planned error categories:

- Transport error (tonic connection/rpc failure)
- Worker error (`WorkerError` returned in envelope)
- Codec error (`serde_json`/`Struct` conversion)
- Protocol error (missing or unexpected `oneof` outcome)

This client intentionally has no mem0-specific methods.

### `crates/memory`: Typed `Mem0Client` (business-specific)

`Mem0Client` becomes the mem0 domain client and exposes typed methods corresponding to mem0 SDK capabilities.

Examples:

- `from_config(...)`
- `add(...)`
- `search(...)`
- `get(...)`
- `get_all(...)`
- `update(...)`
- `history(...)`
- `delete(...)`
- `delete_all(...)`
- `reset(...)`

Implementation pattern:

1. Serialize typed Rust request struct to JSON
2. Call `ExecutionWorkerClient::invoke_json("mem0.<method>", ...)`
3. Deserialize JSON result into typed Rust response struct

All request/response models are strict (no untyped `extra` fields).

## Python Worker Implementation Details

### mem0 capability module

The module stores a single module-level mem0 SDK instance reference.

- `mem0.from_config` constructs/replaces the instance via SDK method `from_config`
- Other handlers require the instance to exist and fail if not configured
- No extra locking or synchronization logic is added

### Registration

Update `python_worker.app.container.build_container()` to register all mem0 capabilities in addition to existing ones.

## Error Handling

### Python worker

- Missing mem0 dependency/import errors bubble up as capability execution failures
- Calling mem0 methods before `mem0.from_config` returns a clear exception (mapped by executor to `WorkerError`)
- SDK exceptions are not wrapped with custom business logic in Python worker

### Rust `Mem0Client`

- Map execution worker `WorkerError` into existing `MemoryError` variants (or a mem0-specific error path under memory error handling)
- Include capability name and underlying worker error message for observability
- Preserve distinction between:
  - execution transport failures
  - remote mem0 SDK failures
  - decode/schema mismatches

## Testing Strategy

### Python worker tests

- Container registration tests verify all `mem0.*` capabilities are registered
- Capability handler tests use monkeypatch/mocks for mem0 SDK object and module import
- gRPC service test verifies `ListCapabilities` includes mem0 capabilities and `Invoke` success path for at least one mem0 method

### Rust tests

- Unit tests for `Struct <-> serde_json` conversions in common execution client
- Unit tests for response envelope decoding and error mapping
- `Mem0Client` unit tests using mocked execution client interface (if trait extraction is introduced)
- Optional integration tests against a running `rara-py-worker` with mem0 installed (deferred if setup is heavy)

## Risks and Mitigations

- mem0 SDK return shapes may vary by version
  - Mitigation: pin mem0 version in Python worker and define strict Rust types against that pinned version
- protobuf `Struct` loses some type fidelity (e.g., integer vs float edge cases)
  - Mitigation: constrain payload/result schemas to JSON-friendly values and test conversions
- Python worker process may handle concurrent requests while mem0 instance is being replaced
  - Accepted for now by explicit requirement: no worker-side locking added

## Rollout Plan (High-Level)

1. Add execution worker Rust gRPC client in `crates/common`
2. Add mem0 capability module + registration in Python worker
3. Refactor `crates/memory::mem0_client` to call execution gRPC client with strict types
4. Update memory manager call sites if method signatures change
5. Add/adjust tests across Python and Rust sides

