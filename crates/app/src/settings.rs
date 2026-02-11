// Copyright 2026 Crrow
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

//! Unified configuration management via the `config` crate.
//!
//! Configuration is loaded from multiple sources with the following priority
//! (highest first):
//!
//! 1. Legacy environment variables (`DATABASE_URL`, `OPENROUTER_API_KEY`, etc.)
//! 2. Prefixed environment variables (`JOB__DATABASE__DATABASE_URL`, etc.)
//! 3. Configuration file (`config.toml` in the working directory)
//! 4. Code defaults

use std::time::Duration;

use job_server::{grpc::GrpcServerConfig, http::RestServerConfig};
use serde::Deserialize;
use yunara_store::config::DatabaseConfig;

use crate::{AppConfig, MinioConfig, OpenRouterConfig};

/// Top-level application settings, deserializable from files and environment
/// variables.
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Database configuration.
    pub database: DatabaseSettings,
    /// HTTP server configuration.
    pub http: RestServerConfig,
    /// gRPC server configuration.
    pub grpc: GrpcServerConfig,
    /// OpenRouter AI service configuration. `None` when the section is absent.
    pub openrouter: Option<OpenRouterSettings>,
    /// MinIO / S3-compatible object store configuration. `None` when absent.
    pub minio: Option<MinioSettings>,
    /// Crawl4AI service configuration.
    pub crawl4ai: Crawl4AiSettings,
    /// Saved-job GC interval in hours.
    pub gc_interval_hours: u64,
    /// Whether to enable graceful shutdown.
    pub enable_graceful_shutdown: bool,
    /// Telegram bot configuration (for combined mode or standalone).
    pub telegram: Option<TelegramSettings>,
    /// Main service HTTP base URL (for telegram bot -> main service calls).
    pub main_service_http_base: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            database:                 DatabaseSettings::default(),
            http:                     RestServerConfig::default(),
            grpc:                     GrpcServerConfig::default(),
            openrouter:               None,
            minio:                    None,
            crawl4ai:                 Crawl4AiSettings::default(),
            gc_interval_hours:        24,
            enable_graceful_shutdown: true,
            telegram:                 None,
            main_service_http_base:   "http://127.0.0.1:3000".to_owned(),
        }
    }
}

/// Database settings with serde-friendly types.
///
/// `Duration` fields are represented as seconds (u64) for easy serialization.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct DatabaseSettings {
    pub database_url:    String,
    pub max_connections: u32,
    pub min_connections: u32,
    /// Connection timeout in seconds.
    pub connect_timeout_secs: u64,
    /// Maximum lifetime of a connection in seconds. 0 = disabled.
    pub max_lifetime_secs: u64,
    /// Idle timeout in seconds. 0 = disabled.
    pub idle_timeout_secs: u64,
}

impl Default for DatabaseSettings {
    fn default() -> Self {
        Self {
            database_url:         "postgres://postgres:postgres@localhost:5432/job".to_owned(),
            max_connections:      10,
            min_connections:      1,
            connect_timeout_secs: 30,
            max_lifetime_secs:    1800,
            idle_timeout_secs:    600,
        }
    }
}

impl DatabaseSettings {
    /// Convert into the infrastructure-layer `DatabaseConfig`.
    #[must_use]
    pub fn into_database_config(self) -> DatabaseConfig {
        let max_lifetime = if self.max_lifetime_secs == 0 {
            None
        } else {
            Some(Duration::from_secs(self.max_lifetime_secs))
        };
        let idle_timeout = if self.idle_timeout_secs == 0 {
            None
        } else {
            Some(Duration::from_secs(self.idle_timeout_secs))
        };

        DatabaseConfig {
            database_url:    self.database_url,
            max_connections: self.max_connections,
            min_connections: self.min_connections,
            connect_timeout: Duration::from_secs(self.connect_timeout_secs),
            max_lifetime,
            idle_timeout,
        }
    }
}

/// OpenRouter AI provider settings.
#[derive(Debug, Clone, Deserialize)]
pub struct OpenRouterSettings {
    pub api_key: String,
    pub model:   String,
}

impl Default for OpenRouterSettings {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            model:   "openai/gpt-4o".to_owned(),
        }
    }
}

/// MinIO / S3-compatible object store settings.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct MinioSettings {
    pub endpoint:   String,
    pub bucket:     String,
    pub access_key: String,
    pub secret_key: String,
    pub region:     String,
}

impl Default for MinioSettings {
    fn default() -> Self {
        Self {
            endpoint:   "http://localhost:9000".to_owned(),
            bucket:     "job-markdown".to_owned(),
            access_key: "minioadmin".to_owned(),
            secret_key: "minioadmin".to_owned(),
            region:     "us-east-1".to_owned(),
        }
    }
}

/// Crawl4AI service settings.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Crawl4AiSettings {
    pub url: String,
}

impl Default for Crawl4AiSettings {
    fn default() -> Self {
        Self {
            url: "http://localhost:11235".to_owned(),
        }
    }
}

/// Telegram bot settings.
#[derive(Debug, Clone, Deserialize)]
pub struct TelegramSettings {
    pub bot_token: String,
    pub chat_id:   i64,
}

impl Settings {
    /// Load settings from config file + environment variables.
    ///
    /// Source priority (highest first):
    /// 1. Legacy environment variables (e.g. `DATABASE_URL`)
    /// 2. `JOB__` prefixed environment variables (e.g. `JOB__DATABASE__DATABASE_URL`)
    /// 3. `config.toml` file in the working directory
    /// 4. Code defaults
    pub fn new() -> Result<Self, config::ConfigError> {
        let builder = config::Config::builder()
            // Layer 1: config file (lowest priority)
            .add_source(config::File::with_name("config").required(false))
            // Layer 2: JOB__-prefixed env vars
            .add_source(
                config::Environment::with_prefix("JOB")
                    .separator("__")
                    .try_parsing(true),
            )
            // Layer 3: legacy env vars (mapped to the Settings structure)
            .set_override_option(
                "database.database_url",
                std::env::var("DATABASE_URL").ok(),
            )?
            .set_override_option(
                "openrouter.api_key",
                std::env::var("OPENROUTER_API_KEY").ok(),
            )?
            .set_override_option(
                "openrouter.model",
                std::env::var("OPENROUTER_MODEL").ok(),
            )?
            .set_override_option("minio.endpoint", std::env::var("MINIO_ENDPOINT").ok())?
            .set_override_option("minio.bucket", std::env::var("MINIO_BUCKET").ok())?
            .set_override_option(
                "minio.access_key",
                std::env::var("MINIO_ACCESS_KEY").ok(),
            )?
            .set_override_option(
                "minio.secret_key",
                std::env::var("MINIO_SECRET_KEY").ok(),
            )?
            .set_override_option("minio.region", std::env::var("MINIO_REGION").ok())?
            .set_override_option("crawl4ai.url", std::env::var("CRAWL4AI_URL").ok())?
            .set_override_option(
                "gc_interval_hours",
                std::env::var("GC_INTERVAL_HOURS").ok(),
            )?
            .set_override_option(
                "telegram.bot_token",
                std::env::var("TELEGRAM_BOT_TOKEN").ok(),
            )?
            .set_override_option(
                "telegram.chat_id",
                std::env::var("TELEGRAM_CHAT_ID").ok(),
            )?
            .set_override_option(
                "main_service_http_base",
                std::env::var("MAIN_SERVICE_HTTP_BASE").ok(),
            )?;

        let cfg = builder.build()?;
        cfg.try_deserialize()
    }

    /// Convert into the application-layer `AppConfig`.
    #[must_use]
    pub fn into_app_config(self) -> AppConfig {
        let db_config = self.database.into_database_config();

        let openrouter = self.openrouter.map(|or| OpenRouterConfig {
            api_key: or.api_key,
            model:   or.model,
        });

        let minio = self.minio.map(|m| MinioConfig {
            endpoint:   m.endpoint,
            bucket:     m.bucket,
            access_key: m.access_key,
            secret_key: m.secret_key,
            region:     m.region,
        });

        AppConfig {
            grpc_config: self.grpc,
            http_config: self.http,
            db_config,
            enable_graceful_shutdown: self.enable_graceful_shutdown,
            openrouter,
            minio,
            crawl4ai_url: self.crawl4ai.url,
            gc_interval_hours: self.gc_interval_hours,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_converts_to_valid_app_config() {
        let settings = Settings::default();
        let app_config = settings.into_app_config();

        assert!(app_config.enable_graceful_shutdown);
        assert!(app_config.openrouter.is_none());
        assert!(app_config.minio.is_none());
        assert_eq!(app_config.gc_interval_hours, 24);
        assert_eq!(app_config.crawl4ai_url, "http://localhost:11235");
    }

    #[test]
    fn database_settings_converts_to_database_config() {
        let db = DatabaseSettings {
            database_url:         "postgres://user:pass@host:5432/mydb".to_owned(),
            max_connections:      20,
            min_connections:      5,
            connect_timeout_secs: 15,
            max_lifetime_secs:    3600,
            idle_timeout_secs:    300,
        };
        let config = db.into_database_config();
        assert_eq!(config.database_url, "postgres://user:pass@host:5432/mydb");
        assert_eq!(config.max_connections, 20);
        assert_eq!(config.min_connections, 5);
    }

    #[test]
    fn database_settings_zero_lifetime_means_none() {
        let db = DatabaseSettings {
            max_lifetime_secs: 0,
            idle_timeout_secs: 0,
            ..DatabaseSettings::default()
        };
        let config = db.into_database_config();
        assert!(config.max_lifetime.is_none());
        assert!(config.idle_timeout.is_none());
    }
}

// Runtime settings cache (DB + RwLock)
use std::sync::{Arc, RwLock};

use job_domain_shared::runtime_settings::{
    RUNTIME_SETTINGS_KV_KEY, RuntimeSettings, RuntimeSettingsPatch,
};
use snafu::{ResultExt, Whatever, whatever};
use yunara_store::KVStore;

#[derive(Clone)]
pub struct RuntimeSettingsService {
    kv:    KVStore,
    cache: Arc<RwLock<RuntimeSettings>>,
}

impl RuntimeSettingsService {
    pub async fn load(kv: KVStore, fallback: RuntimeSettings) -> Result<Self, Whatever> {
        let mut stored = kv
            .get::<RuntimeSettings>(RUNTIME_SETTINGS_KV_KEY)
            .await
            .whatever_context("failed to load runtime settings from kv")?
            .unwrap_or_default();
        stored.normalize();
        let merged = stored.with_fallback(&fallback);
        Ok(Self {
            kv,
            cache: Arc::new(RwLock::new(merged)),
        })
    }

    pub fn current(&self) -> RuntimeSettings {
        self.cache
            .read()
            .map_or_else(|_| RuntimeSettings::default(), |g| g.clone())
    }

    pub async fn update(&self, patch: RuntimeSettingsPatch) -> Result<RuntimeSettings, Whatever> {
        let mut next = self.current();
        next.apply_patch(patch);
        next.normalize();

        self.kv
            .set(RUNTIME_SETTINGS_KV_KEY, &next)
            .await
            .whatever_context("failed to persist runtime settings to kv")?;

        let mut guard = match self.cache.write() {
            Ok(guard) => guard,
            Err(_) => {
                whatever!("failed to lock runtime settings cache")
            }
        };
        *guard = next.clone();
        Ok(next)
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RuntimeSettingsView {
    pub ai:       AiSettingsView,
    pub telegram: TelegramSettingsView,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AiSettingsView {
    pub configured: bool,
    pub model:      Option<String>,
    pub key_hint:   Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TelegramSettingsView {
    pub configured: bool,
    pub chat_id:    Option<i64>,
    pub token_hint: Option<String>,
}

#[must_use]
pub fn to_view(settings: &RuntimeSettings) -> RuntimeSettingsView {
    RuntimeSettingsView {
        ai:       AiSettingsView {
            configured: settings.ai.openrouter_api_key.is_some(),
            model:      settings.ai.model.clone(),
            key_hint:   secret_hint(settings.ai.openrouter_api_key.as_deref()),
        },
        telegram: TelegramSettingsView {
            configured: settings.telegram.bot_token.is_some()
                && settings.telegram.chat_id.is_some(),
            chat_id:    settings.telegram.chat_id,
            token_hint: secret_hint(settings.telegram.bot_token.as_deref()),
        },
    }
}

fn secret_hint(secret: Option<&str>) -> Option<String> {
    let secret = secret?;
    let chars: Vec<char> = secret.chars().collect();
    if chars.is_empty() {
        return None;
    }
    if chars.len() <= 4 {
        return Some("*".repeat(chars.len()));
    }
    let suffix: String = chars[chars.len() - 4..].iter().collect();
    Some(format!("***{suffix}"))
}
