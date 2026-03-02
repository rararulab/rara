# PG Credential Store — Design

## Problem

`codex-oauth` and `mcp/oauth.rs` hardcode `DefaultKeyringStore` (OS keyring) for
persisting OAuth tokens. The OS keyring is unavailable in Kubernetes containers,
so credentials cannot be stored in K8s deployments.

## Decision

1. **New crate `pg-credential-store`** — implements `KeyringStore` trait backed by PostgreSQL.
2. **Keep `keyring-store`** — retains the trait definition + `DefaultKeyringStore` for future local/desktop use.
3. **Always use PG** — PG is available in all environments (local dev and K8s). Simplifies code paths.
4. **Trait becomes async** — `KeyringStore` methods become `async fn`. Enables native sqlx calls in PG impl.

## New Crate: `pg-credential-store`

Location: `crates/integrations/pg-credential-store/`

Dependencies: `keyring-store` (trait), `sqlx` (PG queries), `snafu` (errors).

```rust
pub struct PgKeyringStore {
    pool: PgPool,
}

impl KeyringStore for PgKeyringStore {
    async fn load(&self, service: &str, account: &str) -> Result<Option<String>>;
    async fn save(&self, service: &str, account: &str, value: &str) -> Result<()>;
    async fn delete(&self, service: &str, account: &str) -> Result<bool>;
}
```

## Database Table

Migration in `crates/job-model/migrations/`:

```sql
CREATE TABLE IF NOT EXISTS credential_store (
    service    TEXT NOT NULL,
    account    TEXT NOT NULL,
    value      TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (service, account)
);
```

- `load` → `SELECT value FROM credential_store WHERE service = $1 AND account = $2`
- `save` → `INSERT ... ON CONFLICT (service, account) DO UPDATE SET value = $3, updated_at = now()`
- `delete` → `DELETE FROM credential_store WHERE service = $1 AND account = $2` (return row count > 0)

## KeyringStore Trait — Async Migration

`crates/integrations/keyring-store/src/lib.rs`:

```rust
#[trait_variant::make(Send)]
pub trait KeyringStore: Debug + Send + Sync {
    async fn load(&self, service: &str, account: &str) -> Result<Option<String>>;
    async fn save(&self, service: &str, account: &str, value: &str) -> Result<()>;
    async fn delete(&self, service: &str, account: &str) -> Result<bool>;
}
```

`DefaultKeyringStore` keeps its synchronous keyring calls inside `async fn` bodies (no actual await needed).

## codex-oauth Refactor

All free functions that touch the keyring gain a `store: &dyn KeyringStore` parameter:

- `load_tokens(store)` / `save_tokens(store, tokens)` / `clear_tokens(store)`
- `start_callback_server(store)` — passes `KeyringStoreRef` via axum state to handler
- `handle_callback_inner(query, store)` — uses injected store

`codex-oauth` no longer depends on `keyring` crate or `DefaultKeyringStore`.

## mcp/oauth.rs Refactor

- `OAuthPersistorInner` gains a `store: KeyringStoreRef` field
- `StoredOAuthTokens::load/save/delete` gain a `store: &dyn KeyringStore` parameter
- `load_from_keyring/save_to_keyring/delete_from_keyring` use injected store
- `OAuthCredentialsStoreMode::Keyring` and `Auto` keyring paths use the injected store

## Boot Crate Wiring

`RaraState` gains `pub credential_store: KeyringStoreRef`.

Construction flow:
1. `PgKeyringStore::new(pool.clone())` → `Arc<dyn KeyringStore>`
2. Pass to `build_provider_registry(store)` — loads codex tokens on boot
3. Pass to `init_mcp_manager(store)` — MCP OAuth persistence
4. Pass to `BackendState` — codex OAuth HTTP endpoints

## File Changes

| Change | File |
|--------|------|
| New crate | `crates/integrations/pg-credential-store/` |
| New migration | `crates/job-model/migrations/XXXX_credential_store.sql` |
| Trait async | `crates/integrations/keyring-store/src/lib.rs` |
| Accept injected store | `crates/integrations/codex-oauth/src/lib.rs` |
| Accept injected store | `crates/integrations/mcp/src/oauth.rs` + `client.rs` |
| Wiring | `crates/core/boot/src/providers.rs`, `mcp.rs`, `state.rs` |
| Route state | `crates/extensions/backend-admin/src/settings/codex_oauth.rs` |
