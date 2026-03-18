# rara-sessions — Agent Guidelines

## Purpose

Session metadata persistence layer — provides a file-based session index that tracks session metadata (title, timestamps, user, channel) without storing message content (which lives in tape JSONL files).

## Architecture

### Key modules

- `src/lib.rs` — Crate root, re-exports `file_index` and `types`.
- `src/file_index.rs` — `FileSessionIndex` implementing `rara_kernel::session::SessionIndex` trait. Stores session metadata as JSON files under `rara_paths::sessions_dir()`.
- `src/types.rs` — Re-exported session and message types from `rara-kernel`.
- `src/error.rs` — `snafu`-based error types for session I/O.

### Data flow

1. Kernel creates/updates sessions via the `SessionIndex` trait.
2. `FileSessionIndex` persists each session's metadata as a JSON file at `<sessions_dir>/<session_id>.json`.
3. Session listing scans the directory for all `.json` files and deserializes metadata.
4. Message content is NOT stored here — it lives in the tape system (`rara-kernel::memory`).

## Critical Invariants

- Session metadata files are the source of truth for session existence — if the file is missing, the session does not exist.
- The `SessionIndex` trait is defined in `rara-kernel` — this crate only provides a file-based implementation.
- Session IDs must be valid filesystem names (UUIDs).

## What NOT To Do

- Do NOT store message content in session metadata files — messages belong in tape JSONL files.
- Do NOT access session files directly from other crates — use the `SessionIndex` trait.
- Do NOT assume sessions are in a database — this crate uses the filesystem, not SQLite.

## Dependencies

**Upstream:** `rara-kernel` (for `SessionIndex` trait, session types).

**Downstream:** `rara-app` (creates `FileSessionIndex` during boot and passes to kernel).
