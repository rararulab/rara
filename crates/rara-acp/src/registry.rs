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

//! ACP agent registry — persistent, dynamic agent configuration store.
//!
//! Provides [`AcpRegistry`] (trait) and [`FSAcpRegistry`] (JSON file backend),
//! mirroring the MCP registry pattern in `rara-mcp`.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::{Deserialize, Serialize};
use snafu::ResultExt as _;
use tokio::sync::RwLock;
use tracing::{debug, info};

use crate::error::{self, AcpError};

/// Shared reference to an [`AcpRegistry`] implementation.
pub type AcpRegistryRef = Arc<dyn AcpRegistry>;

/// Registry of ACP agent configurations.
///
/// Implementations may persist to files, databases, or other backends.
#[async_trait::async_trait]
pub trait AcpRegistry: Send + Sync {
    /// Add or update an agent configuration.
    async fn add(&self, name: String, config: AcpAgentConfig) -> Result<(), AcpError>;

    /// Remove an agent configuration. Returns `true` if it existed.
    async fn remove(&self, name: &str) -> Result<bool, AcpError>;

    /// Enable an agent. Returns `true` if it existed.
    async fn enable(&self, name: &str) -> Result<bool, AcpError>;

    /// Disable an agent. Returns `true` if it existed.
    async fn disable(&self, name: &str) -> Result<bool, AcpError>;

    /// List all agent names.
    async fn list(&self) -> Result<Vec<String>, AcpError>;

    /// Get an agent config by name.
    async fn get(&self, name: &str) -> Result<Option<AcpAgentConfig>, AcpError>;

    /// Get all agent configs (enabled and disabled).
    async fn all_agents(&self) -> Result<Vec<(String, AcpAgentConfig)>, AcpError>;

    /// Get all enabled agent configs.
    async fn enabled_agents(&self) -> Result<Vec<(String, AcpAgentConfig)>, AcpError>;
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
    async fn save(&self) -> Result<(), AcpError> {
        let path = self.path.as_ref().ok_or(AcpError::RegistryPathNotSet)?;
        let data = serde_json::to_string_pretty(self).context(error::RegistrySerializeSnafu)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .context(error::RegistrySaveSnafu)?;
        }
        tokio::fs::write(path, data)
            .await
            .context(error::RegistrySaveSnafu)?;
        info!(path = %path.display(), "saved ACP registry");
        Ok(())
    }
}

impl FSAcpRegistry {
    /// Load from a JSON file, or return empty if the file doesn't exist.
    pub async fn load(path: impl AsRef<Path>) -> Result<Self, AcpError> {
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
            .context(error::RegistryLoadSnafu)?;
        let mut inner: FSAcpRegistryInner =
            serde_json::from_str(&data).context(error::RegistryParseSnafu)?;
        inner.path = Some(path.to_path_buf());
        Ok(Self {
            inner: RwLock::new(inner),
        })
    }
}

#[async_trait::async_trait]
impl AcpRegistry for FSAcpRegistry {
    async fn add(&self, name: String, config: AcpAgentConfig) -> Result<(), AcpError> {
        let mut inner = self.inner.write().await;
        // Prevent non-builtin configs from overwriting builtin agents.
        if let Some(existing) = inner.agents.get(&name) {
            if existing.builtin && !config.builtin {
                return Err(AcpError::BuiltinProtection {
                    message: format!("cannot overwrite builtin ACP agent '{name}'"),
                });
            }
        }
        info!(agent = %name, command = %config.command, "adding ACP agent");
        inner.agents.insert(name, config);
        inner.save().await
    }

    async fn remove(&self, name: &str) -> Result<bool, AcpError> {
        let mut inner = self.inner.write().await;
        if let Some(cfg) = inner.agents.get(name) {
            if cfg.builtin {
                return Err(AcpError::BuiltinProtection {
                    message: format!("cannot remove builtin ACP agent '{name}'"),
                });
            }
        }
        let removed = inner.agents.remove(name).is_some();
        if removed {
            info!(agent = %name, "removed ACP agent");
            inner.save().await?;
        }
        Ok(removed)
    }

    async fn enable(&self, name: &str) -> Result<bool, AcpError> {
        let mut inner = self.inner.write().await;
        if let Some(cfg) = inner.agents.get_mut(name) {
            cfg.enabled = true;
            inner.save().await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn disable(&self, name: &str) -> Result<bool, AcpError> {
        let mut inner = self.inner.write().await;
        if let Some(cfg) = inner.agents.get_mut(name) {
            if cfg.builtin {
                return Err(AcpError::BuiltinProtection {
                    message: format!("cannot disable builtin ACP agent '{name}'"),
                });
            }
            cfg.enabled = false;
            inner.save().await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn list(&self) -> Result<Vec<String>, AcpError> {
        let inner = self.inner.read().await;
        Ok(inner.agents.keys().cloned().collect())
    }

    async fn get(&self, name: &str) -> Result<Option<AcpAgentConfig>, AcpError> {
        let inner = self.inner.read().await;
        Ok(inner.agents.get(name).cloned())
    }

    async fn all_agents(&self) -> Result<Vec<(String, AcpAgentConfig)>, AcpError> {
        let inner = self.inner.read().await;
        Ok(inner
            .agents
            .iter()
            .map(|(name, cfg)| (name.clone(), cfg.clone()))
            .collect())
    }

    async fn enabled_agents(&self) -> Result<Vec<(String, AcpAgentConfig)>, AcpError> {
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
// Note: `Default` is required by `#[serde(default)]` for deserialization.
// This is an exception to the project convention against deriving Default on config structs.
#[derive(Debug, Clone, Serialize, Deserialize, Default, bon::Builder)]
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
    #[serde(default = "default_true")]
    #[builder(default = true)]
    pub enabled: bool,
    /// Built-in agents cannot be removed or disabled by users.
    #[serde(default)]
    #[builder(default)]
    pub builtin: bool,
}

fn default_true() -> bool { true }

impl AcpAgentConfig {
    /// Convert to the [`AgentCommand`](crate::AgentCommand) format used by
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
