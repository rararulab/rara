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

//! MCP-specific K8s Pod management.
//!
//! Thin wrapper around [`rara_k8s::PodManager`] with MCP-specific defaults
//! (labels, probes, and container naming). The public API is kept
//! backward-compatible with callers in `managed_client.rs` and `mgr.rs`.

use std::collections::HashMap;

/// Default namespace for MCP pods.
pub const DEFAULT_NAMESPACE: &str = "default";

/// Default container port the MCP server listens on.
pub const DEFAULT_PORT: u16 = 3000;

/// Re-export the underlying error so callers do not need to depend on
/// `rara-k8s` directly.
pub use rara_k8s::K8sError as McpPodError;

// ── McpPodManager ───────────────────────────────────────────────────

/// Manages the lifecycle of ephemeral K8s pods running MCP servers.
///
/// Delegates to [`rara_k8s::PodManager`] and applies MCP-specific defaults
/// (management labels, HTTP health probes).
pub struct McpPodManager {
    inner: rara_k8s::PodManager,
}

impl McpPodManager {
    /// Create a new manager using the default cluster configuration.
    pub async fn new() -> Result<Self, McpPodError> {
        Ok(Self {
            inner: rara_k8s::PodManager::new().await?,
        })
    }

    /// Create a pod for an MCP server and wait until it is running.
    ///
    /// Returns `(pod_name, pod_ip, port)` — the caller uses `pod_ip:port`
    /// to construct an HTTP URL for the streamable HTTP client.
    pub async fn create_mcp_pod(
        &self,
        server_name: &str,
        image: &str,
        namespace: &str,
        port: u16,
        env: &HashMap<String, String>,
        labels: Option<&HashMap<String, String>>,
    ) -> Result<(String, String, u16), McpPodError> {
        let mut all_labels = HashMap::new();
        all_labels.insert(
            "rara.dev/component".to_string(),
            "mcp-server".to_string(),
        );
        all_labels.insert(
            "rara.dev/server-name".to_string(),
            server_name.to_string(),
        );
        if let Some(extra) = labels {
            all_labels.extend(extra.clone());
        }

        let spec = rara_k8s::PodSpec {
            name_prefix: format!("mcp-{server_name}"),
            image: image.to_string(),
            namespace: namespace.to_string(),
            port: Some(port),
            command: None,
            args: None,
            env: env.clone(),
            labels: all_labels,
            resources: None,
            probe: Some(rara_k8s::ProbeSpec {
                http_path: Some("/".to_string()),
                port,
                initial_delay_secs: Some(5),
                period_secs: Some(10),
            }),
            restart_policy: rara_k8s::RestartPolicy::Never,
            timeout_secs: 120,
        };

        let handle = self.inner.create_pod(spec).await?;
        let ip = handle
            .ip
            .ok_or_else(|| rara_k8s::K8sError::NoPodIp {
                name: handle.name.clone(),
            })?;
        Ok((handle.name, ip, handle.port.unwrap_or(port)))
    }

    /// Delete an MCP server pod.
    ///
    /// Silently ignores `NotFound` errors (pod already deleted).
    pub async fn delete_mcp_pod(
        &self,
        pod_name: &str,
        namespace: &str,
    ) -> Result<(), McpPodError> {
        self.inner.delete_pod(pod_name, namespace).await
    }

    /// Check if a pod is still running.
    pub async fn is_pod_running(
        &self,
        pod_name: &str,
        namespace: &str,
    ) -> Result<bool, McpPodError> {
        let status = self.inner.get_pod_status(pod_name, namespace).await?;
        Ok(status.ready)
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_constants() {
        assert_eq!(DEFAULT_NAMESPACE, "default");
        assert_eq!(DEFAULT_PORT, 3000);
    }
}
