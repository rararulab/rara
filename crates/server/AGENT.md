# rara-server — Agent Guidelines

## Purpose

HTTP (Axum) and gRPC (Tonic) server infrastructure — provides configurable server startup, middleware wiring, health checks, metrics, and graceful shutdown via `ServiceHandler`.

## Architecture

### Key modules

- `src/lib.rs` — `ServiceHandler` struct for managing a running server's lifecycle (wait for start, shutdown, wait for stop).
- `src/http.rs` — `RestServerConfig`, `start_rest_server()`, Axum middleware (CORS, body limits, timeout, tracing, Prometheus metrics), health/metrics endpoints.
- `src/grpc.rs` — `GrpcServerConfig`, `GrpcServiceHandler` trait, `start_grpc_server()`, reflection and health service registration.
- `src/grpc/hello.rs` — Example `GrpcServiceHandler` implementation (HelloService).
- `src/dedup_layer.rs` — Request deduplication middleware layer.
- `src/request_key.rs` — Request key extraction for dedup.
- `src/error.rs` — Server-specific error types.

### Data flow

1. `rara-app` calls `start_rest_server(config, route_handlers)` with domain routes.
2. The function builds an Axum router, applies middleware layers (CORS, body limit, timeout, tracing, metrics), binds to the configured address, and spawns a tokio task.
3. Returns `ServiceHandler` for lifecycle control.
4. gRPC follows the same pattern via `start_grpc_server(config, services)`.

### Public API

- `ServiceHandler` — lifecycle handle (wait_for_start, shutdown, wait_for_stop, is_finished).
- `RestServerConfig` / `GrpcServerConfig` — serde-deserializable config structs.
- `start_rest_server()` / `start_grpc_server()` — server launchers.
- `health_routes()` — registers `/api/v1/health`, `/api/health`, `/metrics`.
- `GrpcServiceHandler` trait — implement to register gRPC services.

## Critical Invariants

- `wait_for_start()` consumes the start signal and panics if called twice.
- Middleware layers in Axum only apply to routes registered before `.layer()` — route handlers must be merged before applying layers.
- Shutdown is cooperative via `CancellationToken` — the server drains in-flight requests on cancel.

## What NOT To Do

- Do NOT put business logic or domain routes in this crate — it is infrastructure only. Routes belong in `rara-backend-admin` or `rara-app`.
- Do NOT bypass `ServiceHandler` for shutdown — direct task cancellation skips graceful drain.
- Do NOT hardcode bind addresses — use `RestServerConfig` / `GrpcServerConfig`.

## Dependencies

**Upstream:** `axum`, `tonic`, `tower-http`, `prometheus`, `rara-error`, `rara-api` (protobuf definitions).

**Downstream:** `rara-app` (creates and manages servers).
