//! ACP agent registry — persistent, dynamic agent configuration store.
//!
//! Provides [`AcpRegistry`] (trait) and [`FSAcpRegistry`] (JSON file backend),
//! mirroring the MCP registry pattern in `rara-mcp`.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use smart_default::SmartDefault;
use tokio::sync::RwLock;
use tracing::{debug, info};

/// Shared reference to an [`AcpRegistry`] implementation.
pub type AcpRegistryRef = Arc<dyn AcpRegistry>;

/// Registry of ACP agent configurations.
///
/// Implementations may persist to files, databases, or other backends.
#[async_trait::async_trait]
pub trait AcpRegistry: Send + Sync {
    /// Add or update an agent configuration.
    async fn add(&self, name: String, config: AcpAgentConfig) -> Result<()>;

    /// Remove an agent configuration. Returns `true` if it existed.
    async fn remove(&self, name: &str) -> Result<bool>;

    /// Enable an agent. Returns `true` if it existed.
    async fn enable(&self, name: &str) -> Result<bool>;

    /// Disable an agent. Returns `true` if it existed.
    async fn disable(&self, name: &str) -> Result<bool>;

    /// List all agent names.
    async fn list(&self) -> Result<Vec<String>>;

    /// Get an agent config by name.
    async fn get(&self, name: &str) -> Result<Option<AcpAgentConfig>>;

    /// Get all enabled agent configs.
    async fn enabled_agents(&self) -> Result<Vec<(String, AcpAgentConfig)>>;
}

/// File-system backed [`AcpRegistry`] implementation.
///
/// Persists agent configurations to a JSON file on disk.
/// Uses interior mutability ([`RwLock`]) so that trait methods (`&self`)
/// can update both the in-memory state and the on-disk file atomically.
pub struct FSAcpRegistry {
    inner: RwLock<FSAcpRegistryInner>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct FSAcpRegistryInner {
    #[serde(default)]
    agents: HashMap<String, AcpAgentConfig>,
    #[serde(skip)]
    path:   Option<PathBuf>,
}

impl FSAcpRegistryInner {
    async fn save(&self) -> Result<()> {
        let path = self.path.as_ref().context("no path set for ACP registry")?;
        let data = serde_json::to_string_pretty(self)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(path, data).await?;
        info!(path = %path.display(), "saved ACP registry");
        Ok(())
    }
}

impl FSAcpRegistry {
    /// Load from a JSON file, or return empty if the file doesn't exist.
    pub async fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if !tokio::fs::try_exists(path).await.unwrap_or(false) {
            debug!(path = %path.display(), "ACP registry file not found, using empty");
            return Ok(Self {
                inner: RwLock::new(FSAcpRegistryInner {
                    path: Some(path.to_path_buf()),
                    ..Default::default()
                }),
            });
        }

        let data = tokio::fs::read_to_string(path)
            .await
            .context(format!("failed to read ACP registry: {}", path.display()))?;
        let mut inner: FSAcpRegistryInner = serde_json::from_str(&data)
            .context(format!("failed to parse ACP registry: {}", path.display()))?;
        inner.path = Some(path.to_path_buf());
        Ok(Self {
            inner: RwLock::new(inner),
        })
    }
}

#[async_trait::async_trait]
impl AcpRegistry for FSAcpRegistry {
    async fn add(&self, name: String, config: AcpAgentConfig) -> Result<()> {
        let mut inner = self.inner.write().await;
        // Prevent non-builtin configs from overwriting builtin agents.
        if let Some(existing) = inner.agents.get(&name) {
            if existing.builtin && !config.builtin {
                anyhow::bail!("cannot overwrite builtin ACP agent '{name}'");
            }
        }
        info!(agent = %name, command = %config.command, "adding ACP agent");
        inner.agents.insert(name, config);
        inner.save().await
    }

    async fn remove(&self, name: &str) -> Result<bool> {
        let mut inner = self.inner.write().await;
        if let Some(cfg) = inner.agents.get(name) {
            if cfg.builtin {
                anyhow::bail!("cannot remove builtin ACP agent '{name}'");
            }
        }
        let removed = inner.agents.remove(name).is_some();
        if removed {
            info!(agent = %name, "removed ACP agent");
            inner.save().await?;
        }
        Ok(removed)
    }

    async fn enable(&self, name: &str) -> Result<bool> {
        let mut inner = self.inner.write().await;
        if let Some(cfg) = inner.agents.get_mut(name) {
            cfg.enabled = true;
            inner.save().await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn disable(&self, name: &str) -> Result<bool> {
        let mut inner = self.inner.write().await;
        if let Some(cfg) = inner.agents.get_mut(name) {
            if cfg.builtin {
                anyhow::bail!("cannot disable builtin ACP agent '{name}'");
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
        Ok(inner.agents.keys().cloned().collect())
    }

    async fn get(&self, name: &str) -> Result<Option<AcpAgentConfig>> {
        let inner = self.inner.read().await;
        Ok(inner.agents.get(name).cloned())
    }

    async fn enabled_agents(&self) -> Result<Vec<(String, AcpAgentConfig)>> {
        let inner = self.inner.read().await;
        Ok(inner
            .agents
            .iter()
            .filter(|(_, cfg)| cfg.enabled)
            .map(|(name, cfg)| (name.clone(), cfg.clone()))
            .collect())
    }
}

/// Configuration for a single ACP agent.
#[derive(Debug, Clone, Serialize, Deserialize, SmartDefault, bon::Builder)]
#[serde(default)]
#[builder(on(String, into))]
pub struct AcpAgentConfig {
    /// Command to spawn the agent (e.g. "npx", "gemini").
    pub command: String,
    /// Command-line arguments.
    pub args:    Vec<String>,
    /// Environment variables for the subprocess.
    pub env:     HashMap<String, String>,
    /// Whether this agent is available for use.
    #[default = true]
    #[builder(default = true)]
    pub enabled: bool,
    /// Built-in agents cannot be removed or disabled by users.
    #[serde(default)]
    #[builder(default)]
    pub builtin: bool,
}

impl AcpAgentConfig {
    /// Convert to the [`AgentCommand`] format used by
    /// [`AcpConnection`](crate::AcpConnection).
    pub fn to_agent_command(&self) -> crate::connection::AgentCommand {
        crate::connection::AgentCommand {
            program: self.command.clone(),
            args:    self.args.clone(),
            env:     self
                .env
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        }
    }
}
