# rara-git — Agent Guidelines

## Purpose

Git operations library — provides SSH key management, repository cloning, worktree operations, commit, and push functionality via `git2` (libgit2 bindings).

## Architecture

### Key modules

- `src/repo.rs` — `GitRepo` struct wrapping `git2::Repository`. Provides high-level operations: clone, create worktree, remove worktree, commit, push, branch management.
- `src/ssh.rs` — `SshKeyPair` generation and retrieval. `get_or_create_keypair()` checks `rara_paths` for existing keys, generates Ed25519 keys if missing. `get_public_key()` returns the public key for display.
- `src/error.rs` — `GitError` via `snafu`.

### Public API

- `GitRepo` — stateful repo handle for git operations.
- `get_or_create_keypair()` — idempotent SSH key provisioning.
- `get_public_key()` — retrieve public key string.

## Critical Invariants

- SSH keys are stored under `rara_paths::config_dir()` — do not hardcode paths.
- `git2` operations are blocking — wrap in `tokio::task::spawn_blocking` when called from async code.

## What NOT To Do

- Do NOT shell out to `git` CLI — use `git2` for all git operations.
- Do NOT store SSH private keys in config or database — they live on the filesystem only.

## Dependencies

**Upstream:** `git2`, `rara-paths` (key storage location).

**Downstream:** None currently.
