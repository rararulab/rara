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

//! MCP manager initialization and tool bridging.

use std::sync::Arc;

use rara_kernel::tool::ToolRegistry;
use rara_mcp::{
    manager::{mgr::McpManager, registry::FSMcpRegistry},
    oauth::OAuthCredentialsStoreMode,
    tool_bridge::McpToolBridge,
};
use tracing::info;

use crate::error::{BootError, Result};

/// Initialize the MCP manager from the filesystem registry and start all
/// enabled servers.
pub async fn init_mcp_manager(
    credential_store: rara_keyring_store::KeyringStoreRef,
) -> Result<McpManager> {
    let path = rara_paths::config_dir().join("mcp-servers.json");
    let registry = FSMcpRegistry::load(&path)
        .await
        .map_err(|e| BootError::McpRegistry {
            message: e.to_string(),
        })?;
    let manager = McpManager::new(
        Arc::new(registry),
        OAuthCredentialsStoreMode::default(),
        credential_store,
    );
    let started = manager.start_enabled().await;
    if started.is_empty() {
        info!("no MCP servers to start");
    } else {
        info!(servers = ?started, "MCP servers started");
    }
    Ok(manager)
}

/// Bridge MCP server tools into a [`ToolRegistry`].
pub async fn register_mcp_tools(tool_registry: &mut ToolRegistry, manager: &McpManager) {
    match McpToolBridge::from_manager(manager.clone()).await {
        Ok(bridges) => {
            for bridge in bridges {
                let server = bridge.server_name().to_owned();
                tool_registry.register_mcp(Arc::new(bridge), server);
            }
        }
        Err(e) => tracing::warn!(error = %e, "failed to bridge MCP tools"),
    }
}
