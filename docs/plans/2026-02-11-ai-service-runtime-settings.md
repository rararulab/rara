# AiService Runtime Settings Refactor

## Goal

Make `AiService` depend on `SettingsSvc` so the OpenRouter client is created on-demand from runtime settings, rather than being built once at startup and hot-swapped via `Arc<RwLock<Option<Arc<AiService>>>>`.

## Changes

### 1. `crates/ai/src/service.rs` — AiService

**Before:**
```rust
pub struct AiService {
    client: openrouter::Client,
    default_model: String,
    rate_limiter: Option<RateLimiter>,
}
```

**After:**
```rust
pub struct AiService {
    settings: SettingsSvc,
}
```

- Remove `RateLimiter` entirely (unused).
- Remove `new_client()` constructor. New constructor: `AiService::new(settings: SettingsSvc)`.
- Each agent factory method (`jd_parser()`, `jd_analyzer()`, `job_fit()`, etc.) reads `settings.current()` to get `api_key` + `model`, creates a temporary `openrouter::Client`, and returns an **owned** agent (not borrowing).
- If `api_key` is `None`, return `Err(AiError::NotConfigured)`.

### 2. `crates/ai/src/agents/*.rs` — Owned Agents

All agents change from borrowing (`&'a Client`, `&'a str`) to owning (`Client`, `String`):

```rust
// Before:
pub struct JdParserAgent<'a> {
    client: &'a openrouter::Client,
    model: &'a str,
}

// After:
pub struct JdParserAgent {
    client: openrouter::Client,
    model: String,
}
```

This eliminates lifetime parameters on all agent structs.

### 3. `crates/ai/src/error.rs` — Simplify

- Remove `RateLimited` variant (rate limiter removed).
- Keep `NotConfigured` and `RequestFailed`.

### 4. `crates/ai/Cargo.toml` — New Dependency

Add `job-domain-shared = { workspace = true }`.

### 5. `crates/app/src/lib.rs` — Simplify Composition

- Remove `build_ai_service()` function.
- Remove `Arc<RwLock<Option<Arc<AiService>>>>` — replace with `Arc<AiService>`.
- Create `AiService::new(settings_svc.clone())` directly.
- Remove the settings update callback in routes (no more hot-swap needed).
- Simplify `App` struct: `ai_service_handle` becomes `ai_service: Arc<AiService>`.

### 6. `crates/workers/src/worker_state.rs` — Simplify

```rust
// Before:
pub ai_service_handle: Arc<RwLock<Option<Arc<rara_ai::service::AiService>>>>,

// After:
pub ai_service: Arc<rara_ai::service::AiService>,
```

### 7. `crates/workers/src/saved_job_analyze.rs` — Simplify

Remove the `RwLock<Option<>>` unwrap dance. Just call `state.ai_service.jd_analyzer()` which returns `Result<Agent, AiError>`. Handle `NotConfigured` by skipping.

### 8. `crates/domain/job-tracker/src/bot_internal_routes.rs` — Simplify

Same as workers: `BotInternalState.ai_service: Arc<AiService>` instead of the `Arc<RwLock<Option<>>>` pattern.

### 9. Settings Routes — Simplify

Remove the `on_updated` callback parameter from settings routes. Settings updates just persist to KV — AiService reads fresh settings on next call automatically.

## Files to Modify

1. `crates/ai/Cargo.toml` — add `job-domain-shared` dep
2. `crates/ai/src/error.rs` — remove `RateLimited`
3. `crates/ai/src/service.rs` — rewrite AiService
4. `crates/ai/src/agents/*.rs` (7 files) — owned agents, no lifetime
5. `crates/app/src/lib.rs` — simplify wiring
6. `crates/workers/src/worker_state.rs` — simplify type
7. `crates/workers/src/saved_job_analyze.rs` — simplify usage
8. `crates/domain/job-tracker/src/bot_internal_routes.rs` — simplify usage
9. `crates/domain/shared/src/settings/router.rs` — remove callback if present
