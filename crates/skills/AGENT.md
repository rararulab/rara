# rara-skills — Agent Guidelines

## ClawHub Integration (`clawhub.rs`)

### Architecture

ClawHub is an external skill marketplace at `clawhub.ai`. The integration follows a **client + tool** split:

- `crates/skills/src/clawhub.rs` — `ClawhubClient` (HTTP client, retry logic, zip extraction, manifest registration)
- `crates/app/src/tools/marketplace.rs` — `MarketplaceTool` (wires `clawhub_search`, `clawhub_browse`, `clawhub_install` actions to the client)

`ClawhubClient` is constructed once at boot (`boot.rs`), wrapped in `Arc`, and injected into `MarketplaceTool` via `ToolDeps`.

### Security Invariants (DO NOT WEAKEN)

#### 1. Zip Path Traversal Protection

`extract_zip()` mirrors the tar extraction security in `install.rs`. All three checks are required:

```
1. enclosed_name()        — rejects `..` path components
2. canonicalize + starts_with — verifies resolved path stays inside dest_dir
3. symlink_metadata check — refuses to overwrite existing symlinks
```

Symlinks inside the archive are also skipped entirely.

**Why**: ClawHub is an external registry. A malicious skill package could use path traversal or symlink attacks to write files outside the install directory.

Do NOT:
- Remove the `canonicalize` + `starts_with` check (enclosed_name alone is insufficient)
- Remove the symlink skip or symlink overwrite guard
- Use `.ok()` to silently ignore directory creation failures in zip extraction (use `.context(IoSnafu)?`)

#### 2. Trust Policy: `trusted: false, enabled: false`

All skills installed from ClawHub are marked `trusted: false, enabled: false` in the manifest. This matches the GitHub install behavior in `install.rs`.

**Why**: External marketplace content must not be auto-trusted. Users must explicitly enable skills after reviewing them.

Do NOT:
- Set `trusted: true` or `enabled: true` for ClawHub-sourced skills
- Add an "auto-trust" flag that bypasses this without explicit user confirmation

#### 3. Install Order: Conflict Check → Validate → Download → Cleanup on Failure

`install()` follows this strict sequence:
1. **Conflict check**: If `skill_dir` exists AND is in manifest → reject. If stale (not in manifest) → remove.
2. `get_skill(slug)` validates the slug exists and gets the version.
3. Download and extract.
4. **Cleanup on failure**: If extraction or scan fails, `skill_dir` is removed via `remove_dir_all`.

Do NOT:
- Reorder to download-first-then-query-detail
- Skip the conflict check (prevents silent overwrites of user-modified skills)
- Remove the cleanup-on-failure guard (prevents stale directories on error)

### Error Handling Convention

All errors use **snafu context** with the appropriate `SkillError` variant:

| Operation | Snafu variant |
|-----------|---------------|
| HTTP response parse / body read | `RequestSnafu` |
| Filesystem I/O | `IoSnafu` |
| Zip format / extraction | `ArchiveSnafu` |
| Logical install failures | `InstallSnafu` |
| URL construction failures | `InvalidInput` (only this case) |

Do NOT use `InvalidInput` as a catch-all for network/IO/archive errors — it prevents callers from distinguishing retriable vs fatal failures.

In `marketplace.rs`, errors are converted with `.map_err(anyhow::Error::from)` to preserve the source chain. Do NOT use `.map_err(|e| anyhow::anyhow!("{e}"))` which discards the error type.

### URL Construction

Query parameters use `reqwest::Url::parse_with_params()`. Path segments use `percent_encode_path()` which delegates to `percent_encoding::utf8_percent_encode` with `NON_ALPHANUMERIC`.

Do NOT hand-write percent-encoding functions or use dummy-URL parsing hacks. The `percent-encoding` crate handles edge cases (multibyte UTF-8, reserved characters) correctly.

### Retry Logic

`get_with_retry()` retries on:
- HTTP 429 (rate limit) — respects `Retry-After` header
- 5xx server errors
- Network failures

Constants: `MAX_RETRIES = 3`, `BASE_DELAY_MS = 1500`, `MAX_DELAY_MS = 15000`.

Backoff is exponential (`1500 * 2^attempt`), capped at 15s. When a `Retry-After` header is present, it **replaces** (not adds to) the exponential backoff delay. The delay is computed once per failed attempt via `next_delay_ms` and applied at the top of the next iteration to prevent double-sleeping.

### Serde Models

All response structs use `#[serde(default)]` on optional/non-critical fields. This is intentional — ClawHub's API may add or remove fields, and we must tolerate partial responses.

The three API shapes have different structures:
- **Search** (`/search`): uses `results` array, entries have `score`
- **Browse** (`/skills`): uses `items` array with cursor pagination, entries have nested `stats` and `latestVersion`
- **Detail** (`/skills/{slug}`): wraps skill info in a `skill` object, adds `owner` and `latestVersion` at top level

Do NOT unify these into a single struct — the API shapes are genuinely different.

## GitHub Client (`github.rs`)

`GitHubClient` provides authenticated HTTP access to the GitHub API with automatic retry on transient errors (429 / 5xx). It reads `GITHUB_TOKEN` or `GH_TOKEN` from the environment for authenticated requests (5 000 req/hour vs 60 unauthenticated).

Used by:
- `install.rs` — tarball download and commit SHA lookup
- `marketplace.rs` — GitHub Contents API for marketplace.json / plugin.json

The retry logic mirrors `ClawhubClient::get_with_retry()`: exponential backoff with `Retry-After` header support.

Do NOT:
- Remove the token auth — unauthenticated rate limits (60/hour) break CI and heavy usage
- Duplicate retry logic in callers — use `GitHubClient::get()` instead

## Manifest Locking Invariant

All manifest modifications MUST use `ManifestStore::with_lock()`. This acquires an exclusive `flock()` on a `.lock` file before loading, runs the mutation closure, saves, and releases the lock on drop.

Bare `load()` / `save()` is only acceptable for **read-only** access (e.g. `load_install_manifest()`).

**fs2 / flock() limitation**: advisory only on NFS. Acceptable for single-host CLI usage.

Do NOT:
- Use bare `load()` + `save()` for mutations — causes race conditions between concurrent installs
- Hold the lock across async `.await` points — the closure passed to `with_lock()` must be synchronous
