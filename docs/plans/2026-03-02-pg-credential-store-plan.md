# PG Credential Store Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace hardcoded OS keyring usage with a PostgreSQL-backed `KeyringStore` implementation so credentials persist correctly in K8s.

**Architecture:** New `pg-credential-store` crate implements the existing `KeyringStore` trait against a `credential_store` PG table. The trait is migrated to async. `codex-oauth` and `mcp/oauth.rs` are refactored to accept an injected `KeyringStoreRef` instead of hardcoding `DefaultKeyringStore`. Boot crate creates `PgKeyringStore` and wires it through.

**Tech Stack:** Rust, sqlx (PG), snafu, trait-variant (or native async trait), testcontainers

---

### Task 1: Make KeyringStore trait async

**Files:**
- Modify: `crates/integrations/keyring-store/src/lib.rs` (lines 39-96)
- Modify: `crates/integrations/keyring-store/Cargo.toml`

**Step 1: Update Cargo.toml — remove keyring dep requirement for trait, keep for DefaultKeyringStore**

No Cargo.toml change needed — `keyring` stays since `DefaultKeyringStore` still uses it.

**Step 2: Make trait methods async**

In `crates/integrations/keyring-store/src/lib.rs`, change the trait:

```rust
pub trait KeyringStore: Debug + Send + Sync {
    async fn load(&self, service: &str, account: &str) -> Result<Option<String>>;
    async fn save(&self, service: &str, account: &str, value: &str) -> Result<()>;
    async fn delete(&self, service: &str, account: &str) -> Result<bool>;
}
```

Update `DefaultKeyringStore` impl — prefix each method with `async` but keep the body unchanged (synchronous keyring calls inside async fn are fine):

```rust
impl KeyringStore for DefaultKeyringStore {
    #[tracing::instrument(skip(self), level = "debug")]
    async fn load(&self, service: &str, account: &str) -> Result<Option<String>> {
        // body unchanged
    }

    #[tracing::instrument(skip(self, value), fields(value_len = value.len()), level = "debug")]
    async fn save(&self, service: &str, account: &str, value: &str) -> Result<()> {
        // body unchanged
    }

    #[tracing::instrument(skip(self), level = "debug")]
    async fn delete(&self, service: &str, account: &str) -> Result<bool> {
        // body unchanged
    }
}
```

**Step 3: Verify it compiles**

Run: `cargo check -p rara-keyring-store`
Expected: PASS (native async trait works in edition 2021+ with Rust 1.75+)

**Step 4: Commit**

```bash
git add crates/integrations/keyring-store/
git commit -m "refactor(keyring-store): make KeyringStore trait async"
```

---

### Task 2: Create pg-credential-store crate + migration

**Files:**
- Create: `crates/integrations/pg-credential-store/Cargo.toml`
- Create: `crates/integrations/pg-credential-store/src/lib.rs`
- Create: `crates/rara-model/migrations/20260302000000_credential_store.up.sql`
- Create: `crates/rara-model/migrations/20260302000000_credential_store.down.sql`
- Modify: `Cargo.toml` (workspace root — add member + dep)

**Step 1: Create migration files**

`crates/rara-model/migrations/20260302000000_credential_store.up.sql`:
```sql
CREATE TABLE IF NOT EXISTS credential_store (
    service    TEXT NOT NULL,
    account    TEXT NOT NULL,
    value      TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (service, account)
);
```

`crates/rara-model/migrations/20260302000000_credential_store.down.sql`:
```sql
DROP TABLE IF EXISTS credential_store;
```

**Step 2: Create Cargo.toml**

`crates/integrations/pg-credential-store/Cargo.toml`:
```toml
[package]
name = "rara-pg-credential-store"
version = "0.0.1"
edition.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
homepage.workspace = true
readme.workspace = true
keywords.workspace = true
categories.workspace = true

[lints]
workspace = true

[dependencies]
rara-keyring-store.workspace = true
snafu.workspace = true
sqlx.workspace = true
tracing.workspace = true

[dev-dependencies]
testcontainers = { workspace = true }
testcontainers-modules = { workspace = true, features = ["postgres"] }
tokio = { workspace = true, features = ["test-util", "macros"] }
```

**Step 3: Create lib.rs with PgKeyringStore**

`crates/integrations/pg-credential-store/src/lib.rs`:
```rust
use std::fmt::Debug;

use rara_keyring_store::{KeyringStore, Result};
use sqlx::PgPool;

#[derive(Clone)]
pub struct PgKeyringStore {
    pool: PgPool,
}

impl Debug for PgKeyringStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PgKeyringStore").finish()
    }
}

impl PgKeyringStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl KeyringStore for PgKeyringStore {
    #[tracing::instrument(skip(self), level = "debug")]
    async fn load(&self, service: &str, account: &str) -> Result<Option<String>> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT value FROM credential_store WHERE service = $1 AND account = $2",
        )
        .bind(service)
        .bind(account)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| rara_keyring_store::Error::Pg {
            source: e,
            location: snafu::Location::default(),
        })?;
        Ok(row.map(|(v,)| v))
    }

    #[tracing::instrument(skip(self, value), fields(value_len = value.len()), level = "debug")]
    async fn save(&self, service: &str, account: &str, value: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO credential_store (service, account, value, updated_at) \
             VALUES ($1, $2, $3, now()) \
             ON CONFLICT (service, account) DO UPDATE SET value = $3, updated_at = now()",
        )
        .bind(service)
        .bind(account)
        .bind(value)
        .execute(&self.pool)
        .await
        .map_err(|e| rara_keyring_store::Error::Pg {
            source: e,
            location: snafu::Location::default(),
        })?;
        Ok(())
    }

    #[tracing::instrument(skip(self), level = "debug")]
    async fn delete(&self, service: &str, account: &str) -> Result<bool> {
        let result = sqlx::query(
            "DELETE FROM credential_store WHERE service = $1 AND account = $2",
        )
        .bind(service)
        .bind(account)
        .execute(&self.pool)
        .await
        .map_err(|e| rara_keyring_store::Error::Pg {
            source: e,
            location: snafu::Location::default(),
        })?;
        Ok(result.rows_affected() > 0)
    }
}
```

**Note:** This requires adding a `Pg` variant to `rara_keyring_store::Error`. See next step.

**Step 4: Add Pg error variant to keyring-store Error**

In `crates/integrations/keyring-store/src/lib.rs`, add to the Error enum:

```rust
#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("keyring error: {source}"))]
    Keyring {
        source:   keyring::Error,
        #[snafu(implicit)]
        location: snafu::Location,
    },
    #[snafu(display("database error: {source}"))]
    Pg {
        source:   sqlx::Error,
        #[snafu(implicit)]
        location: snafu::Location,
    },
}
```

Add `sqlx.workspace = true` to `crates/integrations/keyring-store/Cargo.toml` dependencies.

**Step 5: Register in workspace Cargo.toml**

Add to `[workspace.members]` array:
```toml
"crates/integrations/pg-credential-store",
```

Add to `[workspace.dependencies]`:
```toml
rara-pg-credential-store = { path = "crates/integrations/pg-credential-store" }
```

**Step 6: Write integration test**

In `crates/integrations/pg-credential-store/src/lib.rs`, add `#[cfg(test)]` module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;

    async fn setup() -> (PgPool, impl std::any::Any) {
        let container = Postgres::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
        let pool = PgPool::connect(&url).await.unwrap();
        sqlx::migrate!("../../rara-model/migrations")
            .run(&pool)
            .await
            .unwrap();
        (pool, container)
    }

    #[tokio::test]
    async fn load_missing_returns_none() {
        let (pool, _c) = setup().await;
        let store = PgKeyringStore::new(pool);
        assert_eq!(store.load("svc", "acc").await.unwrap(), None);
    }

    #[tokio::test]
    async fn save_then_load() {
        let (pool, _c) = setup().await;
        let store = PgKeyringStore::new(pool);
        store.save("svc", "acc", "secret").await.unwrap();
        assert_eq!(
            store.load("svc", "acc").await.unwrap(),
            Some("secret".to_owned())
        );
    }

    #[tokio::test]
    async fn save_overwrites() {
        let (pool, _c) = setup().await;
        let store = PgKeyringStore::new(pool);
        store.save("svc", "acc", "v1").await.unwrap();
        store.save("svc", "acc", "v2").await.unwrap();
        assert_eq!(
            store.load("svc", "acc").await.unwrap(),
            Some("v2".to_owned())
        );
    }

    #[tokio::test]
    async fn delete_existing_returns_true() {
        let (pool, _c) = setup().await;
        let store = PgKeyringStore::new(pool);
        store.save("svc", "acc", "val").await.unwrap();
        assert!(store.delete("svc", "acc").await.unwrap());
        assert_eq!(store.load("svc", "acc").await.unwrap(), None);
    }

    #[tokio::test]
    async fn delete_missing_returns_false() {
        let (pool, _c) = setup().await;
        let store = PgKeyringStore::new(pool);
        assert!(!store.delete("svc", "acc").await.unwrap());
    }
}
```

**Step 7: Run tests**

Run: `cargo test -p rara-pg-credential-store`
Expected: 5 tests PASS

**Step 8: Commit**

```bash
git add crates/integrations/pg-credential-store/ crates/rara-model/migrations/20260302000000_* crates/integrations/keyring-store/ Cargo.toml
git commit -m "feat(pg-credential-store): PgKeyringStore backed by PostgreSQL"
```

---

### Task 3: Refactor codex-oauth to accept injected KeyringStoreRef

**Files:**
- Modify: `crates/integrations/codex-oauth/src/lib.rs` (lines 32, 111-140, 282-386)
- Modify: `crates/integrations/codex-oauth/Cargo.toml`

**Step 1: Update Cargo.toml**

Remove `rara-keyring-store` from dependencies (codex-oauth no longer needs DefaultKeyringStore directly). Actually — keep it, since we still use `KeyringStore` trait and `KeyringStoreRef` type alias. But the code no longer imports `DefaultKeyringStore`.

**Step 2: Refactor load_tokens / save_tokens / clear_tokens**

Change signatures from taking no store arg to taking `&dyn KeyringStore`:

```rust
use rara_keyring_store::KeyringStore;
// Remove: use rara_keyring_store::DefaultKeyringStore;

pub async fn load_tokens(store: &dyn KeyringStore) -> Result<Option<StoredCodexTokens>, String> {
    let Some(raw) = store
        .load(CODEX_KEYRING_SERVICE, CODEX_TOKEN_ACCOUNT)
        .await
        .map_err(|e| format!("credential store load failed: {e}"))?
    else {
        return Ok(None);
    };
    serde_json::from_str(&raw)
        .map(Some)
        .map_err(|e| e.to_string())
}

pub async fn save_tokens(store: &dyn KeyringStore, tokens: &StoredCodexTokens) -> Result<(), String> {
    let raw = serde_json::to_string(tokens).map_err(|e| e.to_string())?;
    store
        .save(CODEX_KEYRING_SERVICE, CODEX_TOKEN_ACCOUNT, &raw)
        .await
        .map_err(|e| format!("credential store save failed: {e}"))
}

pub async fn clear_tokens(store: &dyn KeyringStore) -> Result<(), String> {
    let _ = store
        .delete(CODEX_KEYRING_SERVICE, CODEX_TOKEN_ACCOUNT)
        .await
        .map_err(|e| format!("credential store delete failed: {e}"))?;
    Ok(())
}
```

**Step 3: Refactor callback server to accept KeyringStoreRef**

Change `start_callback_server` signature:

```rust
use rara_keyring_store::KeyringStoreRef;

pub async fn start_callback_server(store: KeyringStoreRef) -> Result<(), String> {
    // ... (shut down previous server — unchanged) ...

    let app = axum::Router::new().route(
        "/auth/callback",
        axum::routing::get({
            let cancel = cancel_for_handler;
            let store = store.clone();
            move |query: axum::extract::Query<CallbackQuery>| {
                let cancel = cancel.clone();
                let store = store.clone();
                async move { handle_callback(query, cancel, store).await }
            }
        }),
    );
    // rest unchanged ...
}
```

Update handler chain:

```rust
async fn handle_callback(
    axum::extract::Query(query): axum::extract::Query<CallbackQuery>,
    cancel: tokio_util::sync::CancellationToken,
    store: KeyringStoreRef,
) -> axum::response::Redirect {
    // ...
    let redirect_url = match handle_callback_inner(&query, &*store).await {
    // ...
}

async fn handle_callback_inner(query: &CallbackQuery, store: &dyn KeyringStore) -> Result<(), String> {
    // ...
    let tokens = exchange_authorization_code(code, &pending.code_verifier).await?;
    save_tokens(store, &tokens).await?;
    clear_pending_oauth()?;
    // ...
}
```

**Step 4: Verify it compiles**

Run: `cargo check -p rara-codex-oauth`
Expected: PASS (callers will fail — fixed in Task 5)

**Step 5: Commit**

```bash
git add crates/integrations/codex-oauth/
git commit -m "refactor(codex-oauth): accept injected KeyringStoreRef instead of hardcoded DefaultKeyringStore"
```

---

### Task 4: Refactor mcp/oauth.rs to accept injected KeyringStoreRef

**Files:**
- Modify: `crates/integrations/mcp/src/oauth.rs` (lines 26, 47-68, 189-195, 257-275, 279-294, 346-366, 386-412)
- Modify: `crates/integrations/mcp/src/client.rs` (lines 186-217, 612-695)
- Modify: `crates/integrations/mcp/src/manager/mgr.rs` (lines 52-83, 131-148)
- Modify: `crates/integrations/mcp/src/manager/managed_client.rs` (lines 134-158, 378-476)
- Modify: `crates/integrations/mcp/Cargo.toml` (remove `keyring` direct dep)

**Step 1: Refactor OAuthPersistor to hold KeyringStoreRef**

In `oauth.rs`, `OAuthPersistorInner` gains a `store` field:

```rust
struct OAuthPersistorInner {
    server_name:           String,
    url:                   String,
    authorization_manager: Arc<Mutex<AuthorizationManager>>,
    store_mode:            OAuthCredentialsStoreMode,
    store:                 KeyringStoreRef,
    last_credentials:      Mutex<Option<StoredOAuthTokens>>,
}
```

`OAuthPersistor::new()` gains `store: KeyringStoreRef` param. Thread it through to `StoredOAuthTokens::save/delete` calls.

**Step 2: Refactor StoredOAuthTokens load/save/delete**

All methods that currently call `DefaultKeyringStore` directly gain `store: &dyn KeyringStore`:

- `load(server_name, url, store_mode, store)` → passes `store` to `load_from_keyring(server_name, url, store)`
- `save(&self, store_mode, store)` → passes `store` to `save_to_keyring(&self, store)`
- `delete(server_name, url, store_mode, store)` → passes `store` to `delete_from_keyring(server_name, url, store)`

The `_from_keyring` / `_to_keyring` methods replace `let store = DefaultKeyringStore;` with the injected `store` parameter.

All these methods become `async` since `KeyringStore` methods are now async.

**Step 3: Thread KeyringStoreRef through MCP client**

In `client.rs`:
- `new_streamable_http_client()` gains `store: KeyringStoreRef` param
- Passes to `StoredOAuthTokens::load()` and `build_oauth_transport()`
- `build_oauth_transport()` gains `store: KeyringStoreRef`
- `try_oauth_transport()` gains `store: KeyringStoreRef`, passes to `OAuthPersistor::new()`

**Step 4: Thread KeyringStoreRef through McpManager**

In `manager/mgr.rs`:
- `McpManagerInner` gains `store: KeyringStoreRef` field (replaces or complements `store_mode`)
- `McpManager::new()` gains `store: KeyringStoreRef` param
- `start_server()` passes `store` to `AsyncManagedClient::new()`

In `manager/managed_client.rs`:
- `AsyncManagedClient::new()` gains `store: KeyringStoreRef`
- `make_rmcp_client()` gains `store: KeyringStoreRef`, passes to `RmcpClient::new_streamable_http_client()`

**Step 5: Remove direct keyring dependency from mcp Cargo.toml**

Remove line 24: `keyring = { version = "^3.6", features = ["crypto-rust"] }`

Keep `rara-keyring-store` (for the trait).

**Step 6: Verify it compiles**

Run: `cargo check -p rara-mcp`
Expected: PASS (callers in boot crate will fail — fixed in Task 5)

**Step 7: Commit**

```bash
git add crates/integrations/mcp/
git commit -m "refactor(mcp): accept injected KeyringStoreRef for OAuth credential persistence"
```

---

### Task 5: Wire PgKeyringStore in boot crate

**Files:**
- Modify: `crates/core/boot/src/state.rs` (lines 36-47, 64-167)
- Modify: `crates/core/boot/src/providers.rs` (lines 28-30, 68-76)
- Modify: `crates/core/boot/src/mcp.rs` (lines 31-46)
- Modify: `crates/core/boot/Cargo.toml`

**Step 1: Add dependency**

In `crates/core/boot/Cargo.toml`, add:
```toml
rara-pg-credential-store.workspace = true
rara-keyring-store.workspace = true
```

**Step 2: Add credential_store to RaraState**

In `state.rs`:
```rust
pub struct RaraState {
    pub credential_store:    rara_keyring_store::KeyringStoreRef,
    // ... all existing fields ...
}
```

**Step 3: Create PgKeyringStore in RaraState::init()**

At the top of `init()`, after pool is available:

```rust
let credential_store: rara_keyring_store::KeyringStoreRef =
    Arc::new(rara_pg_credential_store::PgKeyringStore::new(pool.clone()));
```

Pass to `build_provider_registry`:
```rust
let provider_registry =
    crate::providers::build_provider_registry(&*settings_provider, &*credential_store).await;
```

Pass to `init_mcp_manager`:
```rust
let mcp_manager = crate::mcp::init_mcp_manager(credential_store.clone())
    .await
    .whatever_context("Failed to initialize MCP manager")?;
```

Add to return struct:
```rust
Ok(Self {
    credential_store,
    // ... rest ...
})
```

**Step 4: Update providers.rs**

Change `build_provider_registry` signature:

```rust
pub async fn build_provider_registry(
    settings: &dyn rara_domain_shared::settings::SettingsProvider,
    credential_store: &dyn rara_keyring_store::KeyringStore,
) -> Arc<rara_kernel::provider::ProviderRegistry> {
```

Update codex section (lines 68-76):
```rust
if let Ok(Some(tokens)) = rara_codex_oauth::load_tokens(credential_store).await {
    // ... rest unchanged ...
}
```

**Step 5: Update mcp.rs**

Change `init_mcp_manager` signature:

```rust
pub async fn init_mcp_manager(
    credential_store: rara_keyring_store::KeyringStoreRef,
) -> Result<McpManager> {
    // ...
    let manager = McpManager::new(
        Arc::new(registry),
        OAuthCredentialsStoreMode::default(),
        credential_store,
    );
    // ...
}
```

**Step 6: Verify full workspace compiles**

Run: `cargo check`
Expected: PASS

**Step 7: Commit**

```bash
git add crates/core/boot/
git commit -m "feat(boot): wire PgKeyringStore as credential store for kernel"
```

---

### Task 6: Final verification — full test suite

**Step 1: Run all tests**

Run: `cargo test --workspace`
Expected: All existing tests + 5 new pg-credential-store tests PASS

**Step 2: Check frontend still builds**

Run: `cd web && npm run build`
Expected: PASS (no frontend changes)

**Step 3: Final commit if any fixes needed**

```bash
git add -A
git commit -m "fix: address test/build issues from credential store migration"
```
