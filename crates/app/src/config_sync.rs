// Copyright 2025 Rararulab
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Bidirectional sync between config.yaml and the settings KV store.
//!
//! [`ConfigFileSync`] watches both directions:
//! - **File -> KV**: `notify` file watcher detects config.yaml edits, flattens
//!   dynamic sections, writes to KV via `batch_update`.
//! - **KV -> File**: subscribes to settings change notifications, debounces
//!   writes, serializes full AppConfig back to YAML.

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
use rara_domain_shared::settings::SettingsProvider;
use rara_vault::VaultClient;
use tokio::sync::{RwLock, mpsc};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::{AppConfig, flatten};

const DEBOUNCE_MS: u64 = 1500;

/// Bidirectional sync between config.yaml and the settings KV store.
pub struct ConfigFileSync {
    settings:          Arc<dyn SettingsProvider>,
    app_config:        Arc<RwLock<AppConfig>>,
    config_path:       PathBuf,
    last_written_hash: Arc<AtomicU64>,
    vault_client:      Option<Arc<VaultClient>>,
}

fn content_hash(content: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    hasher.finish()
}

impl ConfigFileSync {
    /// Create a new ConfigFileSync and perform the initial sync (file -> KV).
    pub async fn new(
        settings: Arc<dyn SettingsProvider>,
        app_config: AppConfig,
        config_path: PathBuf,
        vault_client: Option<Arc<VaultClient>>,
    ) -> anyhow::Result<Self> {
        let sync = Self {
            settings,
            app_config: Arc::new(RwLock::new(app_config)),
            config_path,
            last_written_hash: Arc::new(AtomicU64::new(0)),
            vault_client,
        };
        let initial_config = sync.app_config.read().await.clone();
        sync.sync_from_app_config(&initial_config).await?;
        Ok(sync)
    }

    /// Initial and on-change sync: read config.yaml, flatten dynamic
    /// sections, write to KV store via batch_update.
    async fn sync_from_file(&self) -> anyhow::Result<()> {
        let content = tokio::fs::read_to_string(&self.config_path).await?;
        let new_config: AppConfig = serde_yaml::from_str(&content)?;
        self.sync_from_app_config(&new_config).await
    }

    async fn sync_from_app_config(&self, new_config: &AppConfig) -> anyhow::Result<()> {
        let pairs = flatten::flatten_config_sections(&new_config);
        if !pairs.is_empty() {
            let patches = pairs.into_iter().map(|(k, v)| (k, Some(v))).collect();
            self.settings.batch_update(patches).await?;
        }
        {
            let mut cfg = self.app_config.write().await;
            cfg.llm = new_config.llm.clone();
            cfg.telegram = new_config.telegram.clone();
            cfg.composio = new_config.composio.clone();
            cfg.knowledge = new_config.knowledge.clone();
        }
        info!("config.yaml synced to settings store");
        Ok(())
    }

    /// Write current settings back to config.yaml.
    async fn writeback_to_file(&self) -> anyhow::Result<()> {
        let all_settings = self.settings.list().await;
        let (llm, telegram, composio, knowledge) = flatten::unflatten_from_settings(&all_settings);

        let yaml = {
            let mut cfg = self.app_config.write().await;
            cfg.llm = llm;
            cfg.telegram = telegram;
            cfg.composio = composio;
            cfg.knowledge = knowledge;
            serde_yaml::to_string(&*cfg)?
        };
        let vault_pairs = {
            let cfg = self.app_config.read().await;
            flatten::flatten_config_sections(&cfg)
        };

        let hash = content_hash(&yaml);
        tokio::fs::write(&self.config_path, &yaml).await?;
        self.last_written_hash.store(hash, Ordering::Relaxed);

        if let Some(ref client) = self.vault_client {
            if let Err(e) = client.push_changes(vault_pairs).await {
                warn!(error = %e, "failed to push settings to vault");
            } else {
                debug!("settings pushed to vault");
            }
        }
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
                    if matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
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
                                cfg.composio = new_config.composio;
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
        let mut settings_rx = self.settings.subscribe();

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
                                    Ok(Ok(())) => continue,
                                    Ok(Err(_)) => return,
                                    Err(_) => break,
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::ConfigFileSync;
    use crate::AppConfig;

    const TEST_YAML: &str = r#"
users:
  - name: "testuser"
    role: root
    platforms:
      - type: telegram
        user_id: "12345"

http:
  bind_address: "127.0.0.1:25555"

grpc:
  bind_address: "127.0.0.1:50051"
  server_address: "127.0.0.1:50051"

mita:
  heartbeat_interval: "30m"

llm:
  default_provider: "ollama"
  providers:
    ollama:
      base_url: "http://localhost:11434/v1"
      api_key: "ollama"
      default_model: "qwen3:32b"
      fallback_models:
        - "qwen3:14b"
        - "llama3:8b"

telegram:
  bot_token: "123:ABC"
  chat_id: "456"
  notification_channel_id: "-100"

composio:
  api_key: "cmp_test_key"
  entity_id: "workspace-default"

knowledge:
  embedding_model: "text-embedding-3-small"
  embedding_dimensions: 1536
  search_top_k: 10
  similarity_threshold: 0.85
  extractor_model: "gpt-4o-mini"

gateway:
  repo_url: "https://github.com/example/repo"
  bot_token: "telegram-gateway-token"
  chat_id: 123456
"#;

    #[tokio::test]
    async fn sync_from_file_populates_kv() {
        use rara_domain_shared::settings::SettingsProvider;

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

        let db_store = yunara_store::db::DBStore::new(pool.clone());
        let kv = db_store.kv_store();
        let settings_svc = rara_backend_admin::settings::SettingsSvc::load(kv, pool)
            .await
            .unwrap();
        let settings_provider: Arc<dyn SettingsProvider> = Arc::new(settings_svc);

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
  chat_id: "999"
composio:
  api_key: "cmp_test_key"
  entity_id: "test-entity"
"#;
        tokio::fs::write(&config_path, yaml).await.unwrap();

        let config: AppConfig = serde_yaml::from_str(yaml).unwrap();
        let _sync = ConfigFileSync::new(settings_provider.clone(), config, config_path, None)
            .await
            .unwrap();

        // Verify KV store was populated
        assert_eq!(
            settings_provider
                .get("llm.default_provider")
                .await
                .as_deref(),
            Some("test-provider"),
        );
        assert_eq!(
            settings_provider
                .get("llm.providers.test-provider.base_url")
                .await
                .as_deref(),
            Some("http://localhost:1234"),
        );
        assert_eq!(
            settings_provider.get("telegram.bot_token").await.as_deref(),
            Some("123:ABC"),
        );
        assert_eq!(
            settings_provider.get("telegram.chat_id").await.as_deref(),
            Some("999"),
        );
        assert_eq!(
            settings_provider.get("composio.api_key").await.as_deref(),
            Some("cmp_test_key"),
        );
        assert_eq!(
            settings_provider.get("composio.entity_id").await.as_deref(),
            Some("test-entity"),
        );
    }

    #[tokio::test]
    async fn initial_sync_prefers_bootstrapped_config() {
        use rara_domain_shared::settings::SettingsProvider;

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

        let db_store = yunara_store::db::DBStore::new(pool.clone());
        let kv = db_store.kv_store();
        let settings_svc = rara_backend_admin::settings::SettingsSvc::load(kv, pool)
            .await
            .unwrap();
        let settings_provider: Arc<dyn SettingsProvider> = Arc::new(settings_svc);

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
  default_provider: "local-provider"
  providers:
    local-provider:
      base_url: "http://localhost:1234"
      api_key: "local-key"
      default_model: "local-model"
"#;
        tokio::fs::write(&config_path, yaml).await.unwrap();

        let mut config: AppConfig = serde_yaml::from_str(yaml).unwrap();
        config.llm.as_mut().unwrap().default_provider = Some("vault-provider".into());
        config.llm.as_mut().unwrap().providers.insert(
            "vault-provider".into(),
            crate::flatten::ProviderConfig {
                base_url:        Some("http://vault:1234".into()),
                api_key:         Some("vault-key".into()),
                default_model:   Some("vault-model".into()),
                fallback_models: None,
            },
        );

        let _sync = ConfigFileSync::new(settings_provider.clone(), config, config_path, None)
            .await
            .unwrap();

        assert_eq!(
            settings_provider
                .get("llm.default_provider")
                .await
                .as_deref(),
            Some("vault-provider"),
        );
        assert_eq!(
            settings_provider
                .get("llm.providers.vault-provider.base_url")
                .await
                .as_deref(),
            Some("http://vault:1234"),
        );
    }

    #[tokio::test]
    async fn writeback_without_vault_still_works() {
        use rara_domain_shared::settings::SettingsProvider;

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

        let db_store = yunara_store::db::DBStore::new(pool.clone());
        let kv = db_store.kv_store();
        let settings_svc = rara_backend_admin::settings::SettingsSvc::load(kv, pool)
            .await
            .unwrap();
        let settings_provider: Arc<dyn SettingsProvider> = Arc::new(settings_svc);

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
"#;
        tokio::fs::write(&config_path, yaml).await.unwrap();

        let config: AppConfig = serde_yaml::from_str(yaml).unwrap();
        let sync =
            ConfigFileSync::new(settings_provider.clone(), config, config_path.clone(), None)
                .await
                .unwrap();

        sync.writeback_to_file().await.unwrap();

        let content = tokio::fs::read_to_string(&config_path).await.unwrap();
        assert!(content.contains("test-provider"));
    }

    #[test]
    fn appconfig_yaml_roundtrip() {
        let config: AppConfig = serde_yaml::from_str(TEST_YAML).expect("TEST_YAML should parse");
        let serialized = serde_yaml::to_string(&config).expect("AppConfig should serialize");
        let reparsed: AppConfig =
            serde_yaml::from_str(&serialized).expect("serialized YAML should reparse");

        // Spot-check key fields survived roundtrip
        assert_eq!(config.http.bind_address, reparsed.http.bind_address);
        assert_eq!(
            config
                .llm
                .as_ref()
                .and_then(|l| l.default_provider.as_deref()),
            reparsed
                .llm
                .as_ref()
                .and_then(|l| l.default_provider.as_deref()),
        );
        assert_eq!(
            config
                .telegram
                .as_ref()
                .and_then(|t| t.bot_token.as_deref()),
            reparsed
                .telegram
                .as_ref()
                .and_then(|t| t.bot_token.as_deref()),
        );
        assert_eq!(
            config.composio.as_ref().and_then(|c| c.api_key.as_deref()),
            reparsed
                .composio
                .as_ref()
                .and_then(|c| c.api_key.as_deref()),
        );
        assert_eq!(
            config
                .composio
                .as_ref()
                .and_then(|c| c.entity_id.as_deref()),
            reparsed
                .composio
                .as_ref()
                .and_then(|c| c.entity_id.as_deref()),
        );

        // Duration roundtrip
        assert_eq!(
            config.mita.heartbeat_interval,
            reparsed.mita.heartbeat_interval,
        );

        // Gateway duration roundtrip
        let gw = config.gateway.as_ref().unwrap();
        let gw2 = reparsed.gateway.as_ref().unwrap();
        assert_eq!(gw.check_interval, gw2.check_interval);
        assert_eq!(gw.health_poll_interval, gw2.health_poll_interval);

        // Knowledge roundtrip
        assert_eq!(
            config
                .knowledge
                .as_ref()
                .and_then(|k| k.embedding_model.as_deref()),
            reparsed
                .knowledge
                .as_ref()
                .and_then(|k| k.embedding_model.as_deref()),
        );

        // None fields should stay None
        assert_eq!(
            config
                .telegram
                .as_ref()
                .and_then(|t| t.allowed_group_chat_id.as_deref()),
            reparsed
                .telegram
                .as_ref()
                .and_then(|t| t.allowed_group_chat_id.as_deref()),
        );
    }

    #[test]
    fn appconfig_with_vault_yaml_roundtrip() {
        let yaml = r#"
users:
  - name: "testuser"
    role: root
    platforms:
      - type: telegram
        user_id: "12345"
http:
  bind_address: "127.0.0.1:25555"
grpc:
  bind_address: "127.0.0.1:50051"
  server_address: "127.0.0.1:50051"
mita:
  heartbeat_interval: "30m"
vault:
  address: "http://10.0.0.1:30820"
  mount_path: "secret/rara"
  auth:
    method: approle
    role_id_file: "/etc/rara/vault-role-id"
    secret_id_file: "/etc/rara/vault-secret-id"
  watch_interval: "30s"
  timeout: "5s"
  fallback_to_local: true
"#;
        let config: AppConfig = serde_yaml::from_str(yaml).expect("should parse with vault");
        assert_eq!(
            config.vault.as_ref().map(|vault| vault.address.as_str()),
            Some("http://10.0.0.1:30820"),
        );
        assert_eq!(
            config.vault.as_ref().map(|vault| vault.fallback_to_local),
            Some(true),
        );
        let serialized = serde_yaml::to_string(&config).unwrap();
        let reparsed: serde_yaml::Value = serde_yaml::from_str(&serialized).unwrap();

        assert_eq!(
            reparsed["vault"]["address"].as_str(),
            Some("http://10.0.0.1:30820"),
        );
        assert_eq!(reparsed["vault"]["fallback_to_local"].as_bool(), Some(true));
    }

    #[test]
    fn appconfig_without_vault_parses() {
        let yaml = r#"
users:
  - name: "testuser"
    role: root
    platforms:
      - type: telegram
        user_id: "12345"
http:
  bind_address: "127.0.0.1:25555"
grpc:
  bind_address: "127.0.0.1:50051"
  server_address: "127.0.0.1:50051"
mita:
  heartbeat_interval: "30m"
"#;
        let config: AppConfig = serde_yaml::from_str(yaml).expect("should parse without vault");
        assert!(config.vault.is_none());
        let serialized = serde_yaml::to_string(&config).unwrap();
        let reparsed: serde_yaml::Value = serde_yaml::from_str(&serialized).unwrap();

        assert!(reparsed.get("vault").is_none());
    }
}
