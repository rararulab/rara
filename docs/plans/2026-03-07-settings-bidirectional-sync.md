# Settings Bidirectional Sync Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 实现 config.yaml 与 settings KV store 的双向同步，运行时变更写回文件，文件编辑同步到 settings。

**Architecture:** 新增 `ConfigFileSync` 组件，使用 `notify` crate 监听 config.yaml 变化，使用 `watch::Receiver` 监听 settings 变更。启动时统一用 `sync_from_file()` 替代 `seed_defaults`。回写使用 debounce，回声抑制用 `AtomicU64` content hash。

**Tech Stack:** `notify` (file watcher), `serde_yaml` (serialization), `tokio::sync::watch` (change notification), `std::hash` (content hash)

---

### Task 1: Add `Serialize` to all AppConfig sub-types

**Files:**
- Modify: `crates/app/src/lib.rs:53` — `AppConfig`, `MitaConfig`, `GatewayConfig`, `TelemetryConfig`
- Modify: `crates/app/src/flatten.rs:44,57,75,98` — `LlmConfig`, `ProviderConfig`, `TelegramConfig`, `KnowledgeConfig`
- Modify: `crates/common/yunara-store/src/config.rs:23` — `DatabaseConfig`
- Modify: `crates/symphony/src/config.rs:23` — `SymphonyConfig`
- Modify: `crates/app/src/boot.rs` — `UserConfig`, `PlatformBindingConfig`

**Step 1: Add Serialize derive to AppConfig and its sub-types in lib.rs**

In `crates/app/src/lib.rs`:
- Line 34: add `use serde::{Deserialize, Serialize};` (replace existing `use serde::Deserialize;`)
- Line 53: change `#[derive(Debug, Clone, bon::Builder, Deserialize)]` to `#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]`
- Line 90: `MitaConfig` — add `Serialize`
- Line 98: `GatewayConfig` — add `Serialize`
- Line 133: `TelemetryConfig` — add `Serialize`

**Step 2: Add Serialize to flatten.rs config types**

In `crates/app/src/flatten.rs`:
- Add `use serde::Serialize;` at the top (alongside existing `use serde::Deserialize;`)
- Line 44: `LlmConfig` — add `Serialize`
- Line 57: `ProviderConfig` — add `Serialize`
- Line 75: `TelegramConfig` — add `Serialize`
- Line 98: `KnowledgeConfig` — add `Serialize`

**Step 3: Add Serialize to external crate types**

In `crates/common/yunara-store/src/config.rs`:
- Line 23: change `#[derive(Debug, Clone, bon::Builder, serde::Deserialize)]` to `#[derive(Debug, Clone, bon::Builder, serde::Serialize, serde::Deserialize)]`

In `crates/symphony/src/config.rs`:
- Line 23: change `#[derive(Debug, Clone, Builder, Deserialize)]` to `#[derive(Debug, Clone, Builder, Serialize, Deserialize)]`

In `crates/app/src/boot.rs`:
- `UserConfig` and `PlatformBindingConfig` — add `Serialize`

**Step 4: Verify it compiles**

Run: `cargo check -p rara-app`
Expected: PASS (no errors)

**Step 5: Commit**

```bash
git add -A
git commit -m "refactor(config): add Serialize derive to all AppConfig types

Preparation for bidirectional settings sync — all config types
must be serializable to write back to config.yaml."
```

---

### Task 2: Add `unflatten_from_settings()` to flatten.rs

**Files:**
- Modify: `crates/app/src/flatten.rs` — add `unflatten_from_settings()` function

**Step 1: Write the test**

Add at the bottom of `crates/app/src/flatten.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_flatten_unflatten() {
        let llm = LlmConfig {
            default_provider: Some("openrouter".into()),
            providers: {
                let mut m = HashMap::new();
                m.insert("openrouter".into(), ProviderConfig {
                    base_url: Some("https://openrouter.ai/api/v1".into()),
                    api_key: Some("sk-test".into()),
                    default_model: Some("gpt-4".into()),
                    fallback_models: Some(vec!["gpt-3.5".into()]),
                });
                m
            },
        };
        let tg = TelegramConfig {
            bot_token: Some("123:ABC".into()),
            chat_id: Some("999".into()),
            allowed_group_chat_id: None,
            notification_channel_id: Some("-100123".into()),
        };
        let knowledge = KnowledgeConfig {
            embedding_model: Some("text-embedding-3-small".into()),
            embedding_dimensions: Some(1536),
            search_top_k: Some(10),
            similarity_threshold: Some(0.85),
            extractor_model: Some("gpt-4o-mini".into()),
        };

        // Flatten to KV pairs
        let mut pairs = Vec::new();
        flatten_llm(&llm, &mut pairs);
        flatten_telegram(&tg, &mut pairs);
        flatten_knowledge(&knowledge, &mut pairs);
        let map: HashMap<String, String> = pairs.into_iter().collect();

        // Unflatten back
        let (got_llm, got_tg, got_knowledge) = unflatten_from_settings(&map);

        let got_llm = got_llm.unwrap();
        assert_eq!(got_llm.default_provider.as_deref(), Some("openrouter"));
        let or = &got_llm.providers["openrouter"];
        assert_eq!(or.base_url.as_deref(), Some("https://openrouter.ai/api/v1"));
        assert_eq!(or.api_key.as_deref(), Some("sk-test"));
        assert_eq!(or.default_model.as_deref(), Some("gpt-4"));
        assert_eq!(or.fallback_models.as_deref(), Some(&["gpt-3.5".to_owned()][..]));

        let got_tg = got_tg.unwrap();
        assert_eq!(got_tg.bot_token.as_deref(), Some("123:ABC"));
        assert_eq!(got_tg.chat_id.as_deref(), Some("999"));
        assert_eq!(got_tg.notification_channel_id.as_deref(), Some("-100123"));

        let got_k = got_knowledge.unwrap();
        assert_eq!(got_k.embedding_model.as_deref(), Some("text-embedding-3-small"));
        assert_eq!(got_k.embedding_dimensions, Some(1536));
        assert_eq!(got_k.search_top_k, Some(10));
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p rara-app flatten::tests::roundtrip_flatten_unflatten`
Expected: FAIL — `unflatten_from_settings` not found

**Step 3: Implement unflatten_from_settings**

Add in `crates/app/src/flatten.rs` before the tests module:

```rust
/// Reconstruct config section structs from flat settings KV pairs.
///
/// This is the inverse of `flatten_config_sections()`. Keys without
/// a recognised prefix are ignored.
pub fn unflatten_from_settings(
    pairs: &HashMap<String, String>,
) -> (Option<LlmConfig>, Option<TelegramConfig>, Option<KnowledgeConfig>) {
    let llm = unflatten_llm(pairs);
    let tg = unflatten_telegram(pairs);
    let knowledge = unflatten_knowledge(pairs);
    (llm, tg, knowledge)
}

fn unflatten_llm(pairs: &HashMap<String, String>) -> Option<LlmConfig> {
    let default_provider = pairs.get("llm.default_provider").cloned();

    // Collect provider names from keys like "llm.providers.{name}.{field}"
    let mut providers: HashMap<String, ProviderConfig> = HashMap::new();
    for (key, value) in pairs {
        let rest = match key.strip_prefix("llm.providers.") {
            Some(r) => r,
            None => continue,
        };
        let (name, field) = match rest.split_once('.') {
            Some(pair) => pair,
            None => continue,
        };
        let entry = providers.entry(name.to_owned()).or_default();
        match field {
            "base_url" => entry.base_url = Some(value.clone()),
            "api_key" => entry.api_key = Some(value.clone()),
            "default_model" => entry.default_model = Some(value.clone()),
            "fallback_models" => {
                entry.fallback_models =
                    Some(value.split(',').map(|s| s.trim().to_owned()).collect());
            }
            _ => {}
        }
    }

    if default_provider.is_none() && providers.is_empty() {
        return None;
    }
    Some(LlmConfig {
        default_provider,
        providers,
    })
}

fn unflatten_telegram(pairs: &HashMap<String, String>) -> Option<TelegramConfig> {
    let bot_token = pairs.get("telegram.bot_token").cloned();
    let chat_id = pairs.get("telegram.chat_id").cloned();
    let allowed_group_chat_id = pairs.get("telegram.allowed_group_chat_id").cloned();
    let notification_channel_id = pairs.get("telegram.notification_channel_id").cloned();

    if bot_token.is_none() && chat_id.is_none() && allowed_group_chat_id.is_none() && notification_channel_id.is_none() {
        return None;
    }
    Some(TelegramConfig {
        bot_token,
        chat_id,
        allowed_group_chat_id,
        notification_channel_id,
    })
}

fn unflatten_knowledge(pairs: &HashMap<String, String>) -> Option<KnowledgeConfig> {
    use rara_domain_shared::settings::keys;

    let embedding_model = pairs.get(keys::KNOWLEDGE_EMBEDDING_MODEL).cloned();
    let embedding_dimensions = pairs
        .get(keys::KNOWLEDGE_EMBEDDING_DIMENSIONS)
        .and_then(|v| v.parse().ok());
    let search_top_k = pairs
        .get(keys::KNOWLEDGE_SEARCH_TOP_K)
        .and_then(|v| v.parse().ok());
    let similarity_threshold = pairs
        .get(keys::KNOWLEDGE_SIMILARITY_THRESHOLD)
        .and_then(|v| v.parse().ok());
    let extractor_model = pairs.get(keys::KNOWLEDGE_EXTRACTOR_MODEL).cloned();

    if embedding_model.is_none()
        && embedding_dimensions.is_none()
        && search_top_k.is_none()
        && similarity_threshold.is_none()
        && extractor_model.is_none()
    {
        return None;
    }
    Some(KnowledgeConfig {
        embedding_model,
        embedding_dimensions,
        search_top_k,
        similarity_threshold,
        extractor_model,
    })
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p rara-app flatten::tests::roundtrip_flatten_unflatten`
Expected: PASS

**Step 5: Commit**

```bash
git add -A
git commit -m "feat(config): add unflatten_from_settings() for KV → config struct roundtrip

Inverse of flatten_config_sections(). Reconstructs LlmConfig,
TelegramConfig, and KnowledgeConfig from flat settings KV pairs."
```

---

### Task 3: Add `notify` dependency and create ConfigFileSync skeleton

**Files:**
- Modify: `Cargo.toml` (workspace) — add `notify` to `[workspace.dependencies]`
- Modify: `crates/app/Cargo.toml` — add `notify` and `serde_yaml` dependencies
- Create: `crates/app/src/config_sync.rs`
- Modify: `crates/app/src/lib.rs:15` — add `mod config_sync;`

**Step 1: Add notify to workspace Cargo.toml**

In root `Cargo.toml` under `[workspace.dependencies]`, add:
```toml
notify = "7"
```

**Step 2: Add dependencies to rara-app**

In `crates/app/Cargo.toml`, add:
```toml
notify = { workspace = true }
serde_yaml.workspace = true
```

**Step 3: Create ConfigFileSync skeleton**

Create `crates/app/src/config_sync.rs`:

```rust
//! Bidirectional sync between config.yaml and the settings KV store.
//!
//! [`ConfigFileSync`] watches both directions:
//! - **File → KV**: `notify` file watcher detects config.yaml edits,
//!   flattens dynamic sections, writes to KV via `batch_update`.
//! - **KV → File**: subscribes to settings change notifications,
//!   debounces writes, serializes full AppConfig back to YAML.

use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use rara_backend_admin::settings::SettingsSvc;
use rara_domain_shared::settings::SettingsProvider;
use tokio::sync::{RwLock, mpsc, watch};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::{AppConfig, flatten};

const DEBOUNCE_MS: u64 = 1500;

/// Bidirectional sync between config.yaml and the settings KV store.
pub struct ConfigFileSync {
    settings: SettingsSvc,
    app_config: Arc<RwLock<AppConfig>>,
    config_path: PathBuf,
    last_written_hash: Arc<AtomicU64>,
}

fn content_hash(content: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    hasher.finish()
}

impl ConfigFileSync {
    /// Create a new ConfigFileSync and perform the initial sync (file → KV).
    pub async fn new(
        settings: SettingsSvc,
        app_config: AppConfig,
        config_path: PathBuf,
    ) -> anyhow::Result<Self> {
        let sync = Self {
            settings,
            app_config: Arc::new(RwLock::new(app_config)),
            config_path,
            last_written_hash: Arc::new(AtomicU64::new(0)),
        };
        sync.sync_from_file().await?;
        Ok(sync)
    }

    /// Initial and on-change sync: read config.yaml, flatten dynamic
    /// sections, write to KV store via batch_update.
    async fn sync_from_file(&self) -> anyhow::Result<()> {
        let content = tokio::fs::read_to_string(&self.config_path).await?;
        let new_config: AppConfig = serde_yaml::from_str(&content)?;
        let pairs = flatten::flatten_config_sections(&new_config);
        if !pairs.is_empty() {
            let patches = pairs
                .into_iter()
                .map(|(k, v)| (k, Some(v)))
                .collect();
            self.settings.batch_update(patches).await?;
        }
        // Update in-memory AppConfig with new dynamic sections
        {
            let mut cfg = self.app_config.write().await;
            cfg.llm = new_config.llm;
            cfg.telegram = new_config.telegram;
            cfg.knowledge = new_config.knowledge;
        }
        info!("config.yaml synced to settings store");
        Ok(())
    }

    /// Write current settings back to config.yaml.
    async fn writeback_to_file(&self) -> anyhow::Result<()> {
        let all_settings = self.settings.list().await;
        let (llm, telegram, knowledge) = flatten::unflatten_from_settings(&all_settings);

        let yaml = {
            let mut cfg = self.app_config.write().await;
            cfg.llm = llm;
            cfg.telegram = telegram;
            cfg.knowledge = knowledge;
            serde_yaml::to_string(&*cfg)?
        };

        let hash = content_hash(&yaml);
        tokio::fs::write(&self.config_path, &yaml).await?;
        self.last_written_hash.store(hash, Ordering::Relaxed);
        debug!("settings written back to config.yaml");
        Ok(())
    }

    /// Start both watcher and writeback tasks. Returns when cancelled.
    pub async fn start(self, cancel: CancellationToken) {
        let sync = Arc::new(self);

        let sync_watcher = Arc::clone(&sync);
        let cancel_watcher = cancel.clone();
        let watcher_task = tokio::spawn(async move {
            sync_watcher.run_file_watcher(cancel_watcher).await;
        });

        let sync_writeback = Arc::clone(&sync);
        let cancel_writeback = cancel.clone();
        let writeback_task = tokio::spawn(async move {
            sync_writeback.run_writeback(cancel_writeback).await;
        });

        cancel.cancelled().await;
        watcher_task.abort();
        writeback_task.abort();
    }

    async fn run_file_watcher(self: Arc<Self>, cancel: CancellationToken) {
        let (tx, mut rx) = mpsc::channel::<()>(16);

        let mut watcher = match RecommendedWatcher::new(
            move |res: Result<notify::Event, notify::Error>| {
                if let Ok(event) = res {
                    if matches!(
                        event.kind,
                        EventKind::Modify(_) | EventKind::Create(_)
                    ) {
                        let _ = tx.try_send(());
                    }
                }
            },
            notify::Config::default(),
        ) {
            Ok(w) => w,
            Err(e) => {
                error!(error = %e, "failed to create file watcher");
                return;
            }
        };

        if let Err(e) = watcher.watch(&self.config_path, RecursiveMode::NonRecursive) {
            error!(error = %e, "failed to watch config file");
            return;
        }
        info!(path = %self.config_path.display(), "config file watcher started");

        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                Some(()) = rx.recv() => {
                    // Small delay to let the editor finish writing
                    tokio::time::sleep(Duration::from_millis(100)).await;

                    let content = match tokio::fs::read_to_string(&self.config_path).await {
                        Ok(c) => c,
                        Err(e) => {
                            warn!(error = %e, "failed to read config file after change");
                            continue;
                        }
                    };

                    let hash = content_hash(&content);
                    if hash == self.last_written_hash.load(Ordering::Relaxed) {
                        debug!("config file change is echo from writeback, skipping");
                        continue;
                    }

                    match serde_yaml::from_str::<AppConfig>(&content) {
                        Ok(new_config) => {
                            let pairs = flatten::flatten_config_sections(&new_config);
                            if !pairs.is_empty() {
                                let patches = pairs.into_iter().map(|(k, v)| (k, Some(v))).collect();
                                if let Err(e) = self.settings.batch_update(patches).await {
                                    error!(error = %e, "failed to sync config file changes to settings");
                                    continue;
                                }
                            }
                            {
                                let mut cfg = self.app_config.write().await;
                                cfg.llm = new_config.llm;
                                cfg.telegram = new_config.telegram;
                                cfg.knowledge = new_config.knowledge;
                            }
                            info!("config file change detected and synced to settings");
                        }
                        Err(e) => {
                            warn!(error = %e, "config file has invalid YAML, ignoring");
                        }
                    }
                }
            }
        }
    }

    async fn run_writeback(self: Arc<Self>, cancel: CancellationToken) {
        let mut settings_rx = self.settings.watch_receiver();

        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                res = settings_rx.changed() => {
                    if res.is_err() { break; }

                    // Debounce: wait until no new changes for DEBOUNCE_MS
                    loop {
                        tokio::select! {
                            _ = cancel.cancelled() => return,
                            res = tokio::time::timeout(
                                Duration::from_millis(DEBOUNCE_MS),
                                settings_rx.changed(),
                            ) => {
                                match res {
                                    Ok(Ok(())) => continue, // more changes, reset timer
                                    Ok(Err(_)) => return,   // channel closed
                                    Err(_) => break,        // timeout — debounce done
                                }
                            }
                        }
                    }

                    if let Err(e) = self.writeback_to_file().await {
                        error!(error = %e, "failed to write settings back to config.yaml");
                    }
                }
            }
        }
    }
}
```

**Step 4: Add module declaration**

In `crates/app/src/lib.rs`, add after line 15 (`mod boot;`):
```rust
pub mod config_sync;
```

**Step 5: Verify it compiles**

Run: `cargo check -p rara-app`
Expected: PASS

**Step 6: Commit**

```bash
git add -A
git commit -m "feat(config): add ConfigFileSync for bidirectional config.yaml ↔ settings sync

- File watcher (notify crate) detects config.yaml edits → KV store
- Settings change subscriber → debounce 1.5s → write back to config.yaml
- Echo suppression via AtomicU64 content hash"
```

---

### Task 4: Wire ConfigFileSync into startup, remove seed_defaults

**Files:**
- Modify: `crates/app/src/lib.rs` — replace seed_defaults with ConfigFileSync
- Modify: `crates/extensions/backend-admin/src/settings/service.rs` — remove `seed_defaults` and legacy migration

**Step 1: Update startup in lib.rs**

In `crates/app/src/lib.rs`, replace lines 221-231 (the seed_defaults block):

```rust
    let settings_svc =
        rara_backend_admin::settings::SettingsSvc::load(db_store.kv_store(), pool.clone())
            .await
            .whatever_context("Failed to initialize runtime settings")?;
    let config_defaults = flatten::flatten_config_sections(&config);
    if !config_defaults.is_empty() {
        settings_svc
            .seed_defaults(config_defaults)
            .await
            .whatever_context("Failed to seed config defaults")?;
    }
```

With:

```rust
    let settings_svc =
        rara_backend_admin::settings::SettingsSvc::load(db_store.kv_store(), pool.clone())
            .await
            .whatever_context("Failed to initialize runtime settings")?;

    // Resolve config file path (same logic as AppConfig::new)
    let config_path = {
        let mut path = std::env::current_dir().unwrap_or_default();
        path.push("config.yaml");
        path
    };
    let config_file_sync = config_sync::ConfigFileSync::new(
        settings_svc.clone(),
        config.clone(),
        config_path,
    )
    .await
    .whatever_context("Failed to initialize config file sync")?;
```

Then after `let cancellation_token = CancellationToken::new();` (around line 304), add:

```rust
    // Start bidirectional config ↔ settings sync
    {
        let cancel = cancellation_token.clone();
        tokio::spawn(async move {
            config_file_sync.start(cancel).await;
        });
    }
```

**Step 2: Remove seed_defaults and legacy migration from SettingsSvc**

In `crates/extensions/backend-admin/src/settings/service.rs`:

- In `SettingsSvc::load()` (line 48): remove the `svc.migrate_legacy().await?;` call
- Remove the `migrate_legacy` method (lines 61-90)
- Remove the `seed_defaults` method (lines 95-110)
- Remove the entire `mod legacy` block (lines 195-346)
- Remove the `LEGACY_KV_KEY` constant (line 32)

**Step 3: Verify it compiles**

Run: `cargo check -p rara-app`
Expected: PASS

**Step 4: Verify the full project compiles**

Run: `cargo check`
Expected: PASS

**Step 5: Commit**

```bash
git add -A
git commit -m "feat(config): wire ConfigFileSync into startup, remove seed_defaults

- Replace seed_defaults + legacy migration with ConfigFileSync
- Startup now uses sync_from_file() which batch_updates KV and
  triggers subscriber notifications
- Bidirectional sync runs for app lifetime via CancellationToken"
```

---

### Task 5: Handle serde edge cases for AppConfig serialization

**Files:**
- Modify: `crates/app/src/lib.rs` — add serde attributes for clean YAML output
- Modify: `crates/app/src/flatten.rs` — skip_serializing_if for Option fields

**Step 1: Add skip_serializing_if to Option fields in AppConfig**

In `crates/app/src/lib.rs`, add `#[serde(skip_serializing_if = "Option::is_none")]` to:
- `owner_token`
- `llm`
- `telegram`
- `knowledge`
- `gateway`
- `symphony`

In `crates/app/src/flatten.rs`, add `#[serde(skip_serializing_if = "Option::is_none")]` to all `Option<>` fields in `LlmConfig`, `ProviderConfig`, `TelegramConfig`, `KnowledgeConfig`.

**Step 2: Handle Duration serialization for MitaConfig and GatewayConfig**

`MitaConfig.heartbeat_interval` uses `humantime_serde::deserialize`. Add the symmetric serializer:

```rust
#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct MitaConfig {
    #[serde(
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize",
    )]
    pub heartbeat_interval: Duration,
}
```

Same for `GatewayConfig`:
- `check_interval` — add `serialize_with = "humantime_serde::serialize"`
- `health_poll_interval` — add `serialize_with = "humantime_serde::serialize"`

**Step 3: Verify roundtrip — write a test**

Add to `crates/app/src/config_sync.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appconfig_yaml_roundtrip() {
        let yaml = std::fs::read_to_string("../../config.yaml")
            .expect("config.yaml should exist at repo root");
        let config: AppConfig = serde_yaml::from_str(&yaml)
            .expect("config.yaml should parse");
        let serialized = serde_yaml::to_string(&config)
            .expect("AppConfig should serialize");
        let reparsed: AppConfig = serde_yaml::from_str(&serialized)
            .expect("serialized YAML should reparse");

        // Spot-check key fields survived roundtrip
        assert_eq!(config.http.bind_address, reparsed.http.bind_address);
        assert_eq!(
            config.llm.as_ref().and_then(|l| l.default_provider.as_deref()),
            reparsed.llm.as_ref().and_then(|l| l.default_provider.as_deref()),
        );
    }
}
```

**Step 4: Run test**

Run: `cargo test -p rara-app config_sync::tests::appconfig_yaml_roundtrip`
Expected: PASS

**Step 5: Commit**

```bash
git add -A
git commit -m "fix(config): ensure clean YAML roundtrip for AppConfig

- Add skip_serializing_if for Option fields
- Add humantime_serde::serialize for Duration fields
- Add roundtrip test"
```

---

### Task 6: Integration smoke test

**Files:**
- Modify: `crates/app/src/config_sync.rs` — add integration test

**Step 1: Write integration test for file→KV sync**

Add to `crates/app/src/config_sync.rs` tests module:

```rust
    #[tokio::test]
    async fn sync_from_file_writes_to_kv() {
        // This test needs a real SQLite DB — use a temp file
        let tmp_dir = tempfile::tempdir().unwrap();
        let db_path = tmp_dir.path().join("test.db");
        let db_url = format!("sqlite:{}?mode=rwc", db_path.display());

        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&db_url)
            .await
            .unwrap();
        sqlx::migrate!("../rara-model/migrations")
            .run(&pool)
            .await
            .unwrap();

        let db_store = yunara_store::db::DBStore::from_pool(pool.clone());
        let kv = db_store.kv_store();
        let settings_svc = SettingsSvc::load(kv, pool).await.unwrap();

        // Write a minimal config.yaml to temp dir
        let config_path = tmp_dir.path().join("config.yaml");
        let yaml = r#"
http:
  bind_address: "127.0.0.1:25555"
grpc:
  bind_address: "127.0.0.1:50051"
  server_address: "127.0.0.1:50051"
users:
  - name: test
    role: root
    platforms: []
mita:
  heartbeat_interval: "30m"
llm:
  default_provider: "test-provider"
  providers:
    test-provider:
      base_url: "http://localhost:1234"
      api_key: "test-key"
      default_model: "test-model"
telegram:
  bot_token: "123:ABC"
"#;
        tokio::fs::write(&config_path, yaml).await.unwrap();

        let config: AppConfig = serde_yaml::from_str(yaml).unwrap();
        let sync = ConfigFileSync::new(settings_svc.clone(), config, config_path)
            .await
            .unwrap();

        // Verify KV store was populated
        let provider: &dyn SettingsProvider = &settings_svc;
        assert_eq!(
            provider.get("llm.default_provider").await.as_deref(),
            Some("test-provider"),
        );
        assert_eq!(
            provider.get("telegram.bot_token").await.as_deref(),
            Some("123:ABC"),
        );
    }
```

**Step 2: Run the test**

Run: `cargo test -p rara-app config_sync::tests::sync_from_file_writes_to_kv`
Expected: PASS

**Step 3: Commit**

```bash
git add -A
git commit -m "test(config): add integration test for ConfigFileSync

Verifies that sync_from_file correctly populates the KV store
from a config.yaml file."
```

---

## Summary

| Task | Description | Key Files |
|------|-------------|-----------|
| 1 | Add `Serialize` to all config types | lib.rs, flatten.rs, config.rs, boot.rs |
| 2 | Implement `unflatten_from_settings()` | flatten.rs |
| 3 | Create `ConfigFileSync` component | config_sync.rs (new), Cargo.toml |
| 4 | Wire into startup, remove seed_defaults | lib.rs, service.rs |
| 5 | Handle serde edge cases (Duration, Options) | lib.rs, flatten.rs |
| 6 | Integration smoke test | config_sync.rs |
