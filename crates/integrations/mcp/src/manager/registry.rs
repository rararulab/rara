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

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use smart_default::SmartDefault;
use tokio::sync::RwLock;
use tracing::{debug, info};

pub type McpRegistryRef = Arc<dyn McpRegistry>;

/// Registry of MCP server configurations.
///
/// Implementations may persist to files, databases, or other backends.
/// The gateway frontend UI configures MCP servers through this trait.
#[async_trait::async_trait]
pub trait McpRegistry: Send + Sync {
    /// Add or update a server configuration.
    async fn add(&self, name: String, config: McpServerConfig) -> Result<()>;

    /// Remove a server configuration. Returns `true` if it existed.
    async fn remove(&self, name: &str) -> Result<bool>;

    /// Enable a server. Returns `true` if it existed.
    async fn enable(&self, name: &str) -> Result<bool>;

    /// Disable a server. Returns `true` if it existed.
    async fn disable(&self, name: &str) -> Result<bool>;

    /// List all server names.
    async fn list(&self) -> Result<Vec<String>>;

    /// Get a server config by name.
    async fn get(&self, name: &str) -> Result<Option<McpServerConfig>>;

    /// Get all enabled server configs.
    async fn enabled_servers(&self) -> Result<Vec<(String, McpServerConfig)>>;
}

/// File-system backed [`McpRegistry`] implementation.
///
/// Persists server configurations to a JSON file on disk.
/// Uses interior mutability ([`RwLock`]) so that trait methods (`&self`)
/// can update both the in-memory state and the on-disk file atomically.
pub struct FSMcpRegistry {
    inner: RwLock<FSMcpRegistryInner>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct FSMcpRegistryInner {
    #[serde(default)]
    servers: HashMap<String, McpServerConfig>,
    #[serde(skip)]
    path:    Option<PathBuf>,
}

impl FSMcpRegistryInner {
    async fn save(&self) -> Result<()> {
        let path = self.path.as_ref().context("no path set for MCP registry")?;
        let data = serde_json::to_string_pretty(self)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(path, data).await?;
        info!(path = %path.display(), "saved MCP registry");
        Ok(())
    }
}

impl FSMcpRegistry {
    /// Load from a JSON file, or return empty if the file doesn't exist.
    pub async fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if !tokio::fs::try_exists(path).await.unwrap_or(false) {
            debug!(path = %path.display(), "MCP registry file not found, using empty");
            return Ok(Self {
                inner: RwLock::new(FSMcpRegistryInner {
                    path: Some(path.to_path_buf()),
                    ..Default::default()
                }),
            });
        }

        let data = tokio::fs::read_to_string(path)
            .await
            .context(format!("failed to read MCP registry: {}", path.display()))?;
        let mut inner: FSMcpRegistryInner = serde_json::from_str(&data)
            .context(format!("failed to parse MCP registry: {}", path.display()))?;
        inner.path = Some(path.to_path_buf());
        Ok(Self {
            inner: RwLock::new(inner),
        })
    }
}

#[async_trait::async_trait]
impl McpRegistry for FSMcpRegistry {
    async fn add(&self, name: String, config: McpServerConfig) -> Result<()> {
        let mut inner = self.inner.write().await;
        info!(server = %name, command = %config.command, "adding MCP server");
        inner.servers.insert(name, config);
        inner.save().await
    }

    async fn remove(&self, name: &str) -> Result<bool> {
        let mut inner = self.inner.write().await;
        if let Some(cfg) = inner.servers.get(name) {
            if cfg.builtin {
                anyhow::bail!("cannot remove builtin MCP server '{name}'");
            }
        }
        let removed = inner.servers.remove(name).is_some();
        if removed {
            info!(server = %name, "removed MCP server");
            inner.save().await?;
        }
        Ok(removed)
    }

    async fn enable(&self, name: &str) -> Result<bool> {
        let mut inner = self.inner.write().await;
        if let Some(cfg) = inner.servers.get_mut(name) {
            cfg.enabled = true;
            inner.save().await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn disable(&self, name: &str) -> Result<bool> {
        let mut inner = self.inner.write().await;
        if let Some(cfg) = inner.servers.get_mut(name) {
            if cfg.builtin {
                anyhow::bail!("cannot disable builtin MCP server '{name}'");
            }
            cfg.enabled = false;
            inner.save().await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn list(&self) -> Result<Vec<String>> {
        let inner = self.inner.read().await;
        Ok(inner.servers.keys().cloned().collect())
    }

    async fn get(&self, name: &str) -> Result<Option<McpServerConfig>> {
        let inner = self.inner.read().await;
        Ok(inner.servers.get(name).cloned())
    }

    async fn enabled_servers(&self) -> Result<Vec<(String, McpServerConfig)>> {
        let inner = self.inner.read().await;
        Ok(inner
            .servers
            .iter()
            .filter(|(_, cfg)| cfg.enabled)
            .map(|(name, cfg)| (name.clone(), cfg.clone()))
            .collect())
    }
}

/// Configuration for a single MCP server.
#[derive(Debug, Clone, Serialize, Deserialize, SmartDefault, bon::Builder)]
#[serde(default)]
#[builder(on(String, into))]
pub struct McpServerConfig {
    pub command:   String,
    pub args:      Vec<String>,
    pub env:       HashMap<String, String>,
    #[default = true]
    #[builder(into, default = true)]
    pub enabled:   bool,
    /// Whether this server is a builtin that cannot be removed by users.
    #[serde(default)]
    #[builder(default)]
    pub builtin:   bool,
    pub transport: TransportType,
    /// URL for SSE/HTTP transport. Required when `transport` is `Sse`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url:       Option<String>,
    /// Manual OAuth override (skip discovery/dynamic registration).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth:     Option<McpOAuthConfig>,

    // ── Transport extras ────────────────────────────────────────────
    /// Host environment variable names to forward to a stdio child process.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    pub env_vars:             Vec<String>,
    /// Working directory for a stdio child process.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd:                  Option<PathBuf>,
    /// Environment variable that holds a bearer token (for HTTP transport).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bearer_token_env_var: Option<String>,
    /// Static HTTP headers sent with every request (for HTTP transport).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_headers:         Option<HashMap<String, String>>,
    /// HTTP headers whose values are read from environment variables.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env_http_headers:     Option<HashMap<String, String>>,

    // ── Timeouts ────────────────────────────────────────────────────
    /// Startup (initialize handshake) timeout in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub startup_timeout_secs: Option<u64>,
    /// Per-tool call timeout in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_timeout_secs:    Option<u64>,

    // ── Tool filtering ──────────────────────────────────────────────
    /// Allowlist of tool names. When set, only these tools are exposed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools_enabled:  Option<HashSet<String>>,
    /// Denylist of tool names. These tools are always hidden.
    #[serde(skip_serializing_if = "HashSet::is_empty")]
    #[builder(default)]
    pub tools_disabled: HashSet<String>,
}

/// Transport type for MCP server connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TransportType {
    #[default]
    Stdio,
    Sse,
}

/// Manual OAuth override for MCP servers that don't support standard discovery.
#[derive(Debug, Clone, Serialize, Deserialize, bon::Builder)]
#[builder(on(String, into))]
pub struct McpOAuthConfig {
    pub client_id: String,
    pub auth_url:  String,
    pub token_url: String,
    #[serde(default)]
    #[builder(default)]
    pub scopes:    Vec<String>,
}
