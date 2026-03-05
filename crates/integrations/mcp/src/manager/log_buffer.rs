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
    collections::{HashMap, VecDeque},
    sync::Arc,
};

use serde::Serialize;
use tokio::sync::RwLock;

const MAX_ENTRIES_PER_SERVER: usize = 200;

#[derive(Debug, Clone, Serialize)]
pub struct McpLogEntry {
    pub timestamp: String,
    pub level:     String,
    pub message:   String,
}

/// Per-server ring buffer for MCP log entries.
///
/// Cheap to clone (wraps `Arc<RwLock<...>>`), so it can be shared freely
/// across the manager, client handlers, and the HTTP layer without the
/// outer `McpManager` lock.
#[derive(Debug, Clone, Default)]
pub struct McpLogBuffer {
    inner: Arc<RwLock<HashMap<String, VecDeque<McpLogEntry>>>>,
}

impl McpLogBuffer {
    pub async fn push(&self, server_name: &str, level: &str, message: String) {
        match level {
            "error" => tracing::error!(mcp_server = server_name, "{}", message),
            "warn" => tracing::warn!(mcp_server = server_name, "{}", message),
            "debug" => tracing::debug!(mcp_server = server_name, "{}", message),
            _ => tracing::info!(mcp_server = server_name, "{}", message),
        }
        let mut map = self.inner.write().await;
        let entries = map.entry(server_name.to_string()).or_default();
        if entries.len() >= MAX_ENTRIES_PER_SERVER {
            entries.pop_front();
        }
        entries.push_back(McpLogEntry {
            timestamp: jiff::Timestamp::now().to_string(),
            level: level.to_string(),
            message,
        });
    }

    pub async fn entries(&self, server_name: &str) -> Vec<McpLogEntry> {
        let map = self.inner.read().await;
        map.get(server_name)
            .map(|entries| entries.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub async fn remove(&self, server_name: &str) {
        let mut map = self.inner.write().await;
        map.remove(server_name);
    }
}
