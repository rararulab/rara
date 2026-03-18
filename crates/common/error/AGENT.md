# rara-error — Agent Guidelines

## Purpose

Shared error types and error handling utilities — defines `StatusCode` enum with HTTP/gRPC mappings, `StackError` trait for error chaining, and common network error variants.

## Architecture

### Key module

- `src/lib.rs` — The entire crate. Contains:
  - `StatusCode` enum — `InvalidArgument`, `NotFound`, `Unauthorized`, `Forbidden`, `Conflict`, `Internal`, `Unknown`. Each variant has `http_status()` and `tonic_code()` conversions via `strum` properties.
  - `StackError` trait — error chaining with `debug_fmt()` and `next()`. Enables walking the error chain.
  - `ErrorExt` trait — extends `StackError` with `status_code()`, `output_msg()`, `root_cause()`.
  - `Error` / `NetworkError` — concrete error types for connection and address parsing failures.
  - `Result<T>` type alias.

## Critical Invariants

- All error types in the workspace should use `snafu` — do not use manual `impl Display + impl Error`.
- `StatusCode` mappings must stay in sync with HTTP and gRPC conventions.
- `output_msg()` hides internal details for `Internal`/`Unknown` status codes — do not expose stack traces to users.

## What NOT To Do

- Do NOT add domain-specific errors here — this crate is for shared infrastructure errors only.
- Do NOT use manual `Display + Error` implementations — use `snafu`.
- Do NOT expose internal error details in user-facing `output_msg()` for `Internal`/`Unknown` codes.

## Dependencies

**Upstream:** `http` (StatusCode), `snafu`, `strum`, `tonic` (gRPC Code), `serde`.

**Downstream:** `rara-server`, `rara-app`, and most workspace crates.
