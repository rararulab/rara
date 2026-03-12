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

#### 3. Install Order: Validate Before Download

`install()` calls `get_skill(slug)` **before** downloading. This:
- Validates the slug exists on ClawHub
- Retrieves the version number
- Fails fast on invalid slugs without downloading anything

Do NOT reorder to download-first-then-query-detail.

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

Query parameters use `reqwest::Url::parse_with_params()`. Path segments use `percent_encode_path()`.

Do NOT hand-write percent-encoding functions. The `reqwest` / `url` crate handles edge cases (multibyte UTF-8, reserved characters) correctly.

### Retry Logic

`get_with_retry()` retries on:
- HTTP 429 (rate limit) — respects `Retry-After` header
- 5xx server errors
- Network failures

Constants: `MAX_RETRIES = 3`, `BASE_DELAY_MS = 1500`, `MAX_DELAY_MS = 15000`.

Backoff is exponential (`1500 * 2^attempt`), capped at 15s. The `Retry-After` header overrides the calculated delay when present.

### Serde Models

All response structs use `#[serde(default)]` on optional/non-critical fields. This is intentional — ClawHub's API may add or remove fields, and we must tolerate partial responses.

The three API shapes have different structures:
- **Search** (`/search`): uses `results` array, entries have `score`
- **Browse** (`/skills`): uses `items` array with cursor pagination, entries have nested `stats` and `latestVersion`
- **Detail** (`/skills/{slug}`): wraps skill info in a `skill` object, adds `owner` and `latestVersion` at top level

Do NOT unify these into a single struct — the API shapes are genuinely different.
