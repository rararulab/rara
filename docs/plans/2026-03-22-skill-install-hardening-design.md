# Skill Install Hardening — Design

## Goal

Harden the skill installation system against concurrent corruption, GitHub rate limiting, and blocking IO on the async runtime.

## Approach

Three orthogonal improvements applied to `crates/skills/`:

1. **Manifest file locking** — `ManifestStore::with_lock()` wraps load-mutate-save in an exclusive `flock()` (via `fs2`). Prevents two concurrent `install_skill` / `remove_repo` calls from clobbering each other's manifest writes.

2. **Centralized GitHub client** — `GitHubClient` in `github.rs` reads `GITHUB_TOKEN` / `GH_TOKEN` from the environment and adds `Authorization: Bearer` headers. Retries on 429 / 5xx with exponential backoff. Replaces ad-hoc `reqwest::Client::new()` + hardcoded User-Agent strings across `install.rs` and `marketplace.rs`.

3. **Async IO fixes** — Replace `std::fs::remove_dir_all` with `tokio::fs::remove_dir_all` in `clawhub.rs::install()` to avoid blocking the tokio runtime during cleanup.

## Affected Crates

- `crates/skills/` only. No changes to kernel, app, or other crates.

## Key Decisions

- **flock() over fcntl()**: `fs2` uses `flock()` which is advisory-only on NFS. Acceptable because rara is a single-host CLI tool.
- **Lock file is separate**: `skills-manifest.json.lock` rather than locking the manifest file itself, to avoid interference with atomic write (tmp + rename) strategy.
- **GitHubClient is constructed per-call**: Lightweight (just reads env vars), avoids threading an `Arc<GitHubClient>` through the call graph. Token is read once per client construction.
- **ClawHub keeps its own retry**: `ClawhubClient::get_with_retry()` talks to `clawhub.ai`, not GitHub. Only GitHub API callers use `GitHubClient`.

## Implementation Steps

1. Add `fs2` dependency, implement `ManifestStore::with_lock()`
2. Migrate all manifest mutation sites to `with_lock()`
3. Create `github.rs` with `GitHubClient`
4. Update `install.rs` to use `GitHubClient`
5. Update `marketplace.rs::fetch_index` to use `GitHubClient`
6. Fix blocking `std::fs::remove_dir_all` in `clawhub.rs`
7. Update `AGENT.md` with new invariants
