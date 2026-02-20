// Copyright 2025 Crrow
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

//! McpManager: lifecycle management for multiple MCP server connections.

use std::{collections::HashMap, sync::Arc};

use anyhow::{Context, Result};
use rmcp::model::{
    CallToolResult, ListResourcesResult, ReadResourceRequestParams, ReadResourceResult, Tool,
};
use serde_json::Value;
use tokio::sync::RwLock;
use tracing::{info, instrument, warn};

use crate::{
    manager::{
        erm::ElicitationRequestManager,
        log_buffer::McpLogBuffer,
        managed_client::AsyncManagedClient,
        registry::{McpRegistryRef, McpServerConfig},
    },
    oauth::OAuthCredentialsStoreMode,
};

/// Manages the lifecycle of multiple MCP server connections.
#[derive(Clone)]
pub struct McpManager {
    inner:      Arc<RwLock<McpManagerInner>>,
    /// Per-server log ring buffer.  Lives outside the `RwLock` because
    /// `McpLogBuffer` carries its own `Arc<RwLock<…>>` internally.
    log_buffer: McpLogBuffer,
}

struct McpManagerInner {
    clients:              HashMap<String, AsyncManagedClient>,
    elicitation_requests: ElicitationRequestManager,
    registry:             McpRegistryRef,
    store_mode:           OAuthCredentialsStoreMode,
}

impl McpManager {
    #[instrument(skip_all)]
    pub fn new(registry: McpRegistryRef, store_mode: OAuthCredentialsStoreMode) -> Self {
        Self {
            inner:      Arc::new(RwLock::new(McpManagerInner {
                clients: HashMap::new(),
                elicitation_requests: ElicitationRequestManager::default(),
                registry,
                store_mode,
            })),
            log_buffer: McpLogBuffer::default(),
        }
    }

    /// Return a reference to the per-server log buffer.
    pub fn log_buffer(&self) -> &McpLogBuffer { &self.log_buffer }

    /// Start all enabled servers from the registry concurrently.
    ///
    /// Each server's startup runs in parallel via
    /// [`futures::future::join_all`]. Individual failures are logged but do
    /// not prevent other servers from starting.
    #[instrument(skip(self))]
    pub async fn start_enabled(&self) -> Vec<String> {
        let enabled = {
            let inner = self.inner.read().await;
            match inner.registry.enabled_servers().await {
                Ok(servers) => servers,
                Err(e) => {
                    warn!(error = %e, "failed to load enabled servers from registry");
                    return Vec::new();
                }
            }
        };

        let futs: Vec<_> = enabled
            .into_iter()
            .map(|(name, config)| async move {
                match self.start_server(&name, &config).await {
                    Ok(()) => Some(name),
                    Err(e) => {
                        warn!(server = %name, error = %e, "failed to start MCP server");
                        None
                    }
                }
            })
            .collect();

        futures::future::join_all(futs)
            .await
            .into_iter()
            .flatten()
            .collect()
    }

    /// Start a single server connection.
    ///
    /// Creates an [`AsyncManagedClient`] that lazily performs the MCP
    /// handshake, stores it immediately (so other callers can observe
    /// the pending state), then awaits startup completion.
    #[instrument(skip(self, config), fields(server = %name))]
    pub async fn start_server(&self, name: &str, config: &McpServerConfig) -> Result<()> {
        self.stop_server(name).await;

        let (store_mode, erm) = {
            let inner = self.inner.read().await;
            (inner.store_mode, inner.elicitation_requests.clone())
        };

        let managed = AsyncManagedClient::new(
            name,
            config.clone(),
            store_mode,
            erm,
            self.log_buffer.clone(),
        );

        self.log_buffer
            .push(name, "info", "connecting...".into())
            .await;

        // Store immediately so concurrent callers can await the same startup.
        {
            let mut inner = self.inner.write().await;
            inner.clients.insert(name.to_string(), managed.clone());
        }

        // Wait for startup to finish.
        if let Err(e) = managed.client().await {
            self.log_buffer
                .push(name, "error", format!("connection failed: {e}"))
                .await;
            let mut inner = self.inner.write().await;
            inner.clients.remove(name);
            return Err(anyhow::anyhow!("{e}"));
        }

        info!(server = %name, "MCP server started");
        Ok(())
    }

    /// Stop a server connection.
    #[instrument(skip(self), fields(server = %name))]
    pub async fn stop_server(&self, name: &str) {
        let client = {
            let mut inner = self.inner.write().await;
            inner.clients.remove(name)
        };
        if let Some(client) = client {
            client.cancel();
            self.log_buffer
                .push(name, "info", "disconnected".into())
                .await;
        }
    }

    /// Restart a server.
    #[instrument(skip(self), fields(server = %name))]
    pub async fn restart_server(&self, name: &str) -> Result<()> {
        let config = {
            let inner = self.inner.read().await;
            inner
                .registry
                .get(name)
                .await?
                .with_context(|| format!("MCP server '{name}' not found in registry"))?
        };
        self.start_server(name, &config).await
    }

    /// Shut down all servers concurrently.
    ///
    /// Drains all clients in a single write-lock acquisition, then cancels
    /// them all. Much cheaper than calling [`stop_server`](Self::stop_server)
    /// in a loop (which would acquire/release the lock N times).
    #[instrument(skip(self))]
    pub async fn shutdown_all(&self) {
        let clients: Vec<AsyncManagedClient> = {
            let mut inner = self.inner.write().await;
            inner.clients.drain().map(|(_, c)| c).collect()
        };
        for client in clients {
            client.cancel();
        }
    }

    // ── Registry operations ─────────────────────────────────────────

    /// Add a server to the registry and optionally start it.
    #[instrument(skip(self, config), fields(server = %name))]
    pub async fn add_server(
        &self,
        name: String,
        config: McpServerConfig,
        start: bool,
    ) -> Result<()> {
        let enabled = config.enabled;
        {
            let inner = self.inner.read().await;
            inner.registry.add(name.clone(), config.clone()).await?;
        }
        if start && enabled {
            self.start_server(&name, &config).await?;
        }
        Ok(())
    }

    /// Remove a server from the registry and stop it.
    #[instrument(skip(self), fields(server = %name))]
    pub async fn remove_server(&self, name: &str) -> Result<bool> {
        self.stop_server(name).await;
        let inner = self.inner.read().await;
        let result = inner.registry.remove(name).await;
        drop(inner);
        result
    }

    /// Enable a server and start it.
    #[instrument(skip(self), fields(server = %name))]
    pub async fn enable_server(&self, name: &str) -> Result<bool> {
        let config = {
            let inner = self.inner.read().await;
            if !inner.registry.enable(name).await? {
                return Ok(false);
            }
            inner.registry.get(name).await?
        };
        if let Some(config) = config {
            self.start_server(name, &config).await?;
        }
        Ok(true)
    }

    /// Disable a server and stop it.
    #[instrument(skip(self), fields(server = %name))]
    pub async fn disable_server(&self, name: &str) -> Result<bool> {
        self.stop_server(name).await;
        let inner = self.inner.read().await;
        let result = inner.registry.disable(name).await;
        drop(inner);
        result
    }

    /// Update a server's configuration and restart it if running.
    #[instrument(skip(self, config), fields(server = %name))]
    pub async fn update_server(&self, name: &str, config: McpServerConfig) -> Result<()> {
        let was_running = {
            let inner = self.inner.read().await;
            inner.clients.contains_key(name)
        };
        {
            let inner = self.inner.read().await;
            let existing = inner.registry.get(name).await?;
            let enabled = existing.as_ref().is_none_or(|c| c.enabled);
            let mut new_config = config;
            new_config.enabled = enabled;
            inner.registry.add(name.to_string(), new_config).await?;
        }
        if was_running {
            self.restart_server(name).await?;
        }
        Ok(())
    }

    /// Get the registry reference (for use in routes, etc.).
    #[instrument(skip(self))]
    pub async fn registry(&self) -> McpRegistryRef { Arc::clone(&self.inner.read().await.registry) }

    // ── MCP operations ──────────────────────────────────────────────

    /// List tools advertised by a connected server.
    ///
    /// Returns the filtered tool list (respecting `tools_enabled` /
    /// `tools_disabled` from the server config). Returns an error if
    /// the server is not connected.
    #[instrument(skip(self), fields(server = %name))]
    pub async fn list_tools(&self, name: &str) -> Result<Vec<Tool>> {
        let managed = self.get_managed_client(name).await?;
        let mc = managed.client().await.map_err(|e| anyhow::anyhow!("{e}"))?;
        let tools = mc.list_tools().await?;
        Ok(tools
            .into_iter()
            .filter(|t| mc.tool_filter.allowed(&t.tool_name))
            .map(|t| t.tool.clone())
            .collect())
    }

    /// Call a tool on a connected server.
    #[instrument(skip(self, arguments), fields(server = %server_name, tool = %tool_name))]
    pub async fn call_tool(
        &self,
        server_name: &str,
        tool_name: &str,
        arguments: Option<Value>,
    ) -> Result<CallToolResult> {
        let managed = self.get_managed_client(server_name).await?;
        let mc = managed.client().await.map_err(|e| anyhow::anyhow!("{e}"))?;
        let timeout = mc.tool_timeout;
        let result = mc
            .client
            .call_tool(tool_name.to_string(), arguments, timeout)
            .await;
        drop(mc);
        result
    }

    /// List resources advertised by a connected server.
    #[instrument(skip(self), fields(server = %name))]
    pub async fn list_resources(&self, name: &str) -> Result<ListResourcesResult> {
        let managed = self.get_managed_client(name).await?;
        let mc = managed.client().await.map_err(|e| anyhow::anyhow!("{e}"))?;
        let result = mc.client.list_resources(None, mc.tool_timeout).await;
        drop(mc);
        result
    }

    /// Read a resource from a connected server.
    #[instrument(skip(self, params), fields(server = %name))]
    pub async fn read_resource(
        &self,
        name: &str,
        params: ReadResourceRequestParams,
    ) -> Result<ReadResourceResult> {
        let managed = self.get_managed_client(name).await?;
        let mc = managed.client().await.map_err(|e| anyhow::anyhow!("{e}"))?;
        let result = mc.client.read_resource(params, mc.tool_timeout).await;
        drop(mc);
        result
    }

    /// Return the names of all currently connected servers.
    #[instrument(skip(self))]
    pub async fn connected_servers(&self) -> Vec<String> {
        self.inner.read().await.clients.keys().cloned().collect()
    }

    // ── Private helpers ─────────────────────────────────────────────

    async fn get_managed_client(&self, name: &str) -> Result<AsyncManagedClient> {
        let inner = self.inner.read().await;
        inner
            .clients
            .get(name)
            .cloned()
            .with_context(|| format!("MCP server '{name}' is not connected"))
    }
}
