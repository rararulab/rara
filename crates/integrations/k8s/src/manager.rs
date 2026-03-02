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

//! Core Pod lifecycle manager.
//!
//! [`PodManager`] creates, deletes, and inspects ephemeral K8s pods.
//! It is intentionally generic — no MCP or agent logic here.

use std::time::Duration;

use k8s_openapi::api::core::v1 as k8s_core;
use kube::{
    Api, Client,
    api::{DeleteParams, LogParams, PostParams},
    runtime::wait::{await_condition, conditions::is_pod_running},
};
use tracing::{debug, info};

use crate::{
    error::K8sError,
    types::{PodHandle, PodStatus},
};

/// Manages the lifecycle of ephemeral K8s pods.
pub struct PodManager {
    client: Client,
}

impl PodManager {
    /// Create a new manager using the default cluster configuration.
    ///
    /// Uses in-cluster config when running inside K8s, or the local
    /// kubeconfig (`~/.kube/config`) on a developer machine.
    pub async fn new() -> Result<Self, K8sError> {
        let client = Client::try_default()
            .await
            .map_err(|source| K8sError::KubeClient { source })?;
        Ok(Self { client })
    }

    /// Create a manager from an existing [`kube::Client`] (for connection
    /// reuse).
    pub fn with_client(client: Client) -> Self { Self { client } }

    /// Create a pod and wait until it reaches the `Running` phase.
    ///
    /// Accepts a raw [`k8s_core::Pod`] — callers have full control over the
    /// spec. `namespace` determines where the pod is created. `timeout` is
    /// how long to wait for Running state.
    pub async fn create_pod(
        &self,
        pod: k8s_core::Pod,
        namespace: &str,
        timeout: Duration,
    ) -> Result<PodHandle, K8sError> {
        let pod_name = pod
            .metadata
            .name
            .clone()
            .unwrap_or_else(|| generate_pod_name("pod"));

        let pods: Api<k8s_core::Pod> = Api::namespaced(self.client.clone(), namespace);

        debug!(pod = %pod_name, namespace = %namespace, "creating pod");

        pods.create(&PostParams::default(), &pod)
            .await
            .map_err(|source| K8sError::KubeClient { source })?;

        // Wait for the pod to reach Running state.
        let running = await_condition(pods.clone(), &pod_name, is_pod_running());
        let result = tokio::time::timeout(timeout, running).await;

        match result {
            Ok(Ok(Some(pod_obj))) => {
                let ip = pod_obj.status.as_ref().and_then(|s| s.pod_ip.clone());

                let port = pod_obj
                    .spec
                    .as_ref()
                    .and_then(|s| s.containers.first())
                    .and_then(|c| c.ports.as_ref())
                    .and_then(|p| p.first())
                    .map(|p| p.container_port as u16);

                info!(pod = %pod_name, ip = ?ip, "pod is running");

                Ok(PodHandle {
                    name: pod_name,
                    namespace: namespace.to_string(),
                    ip,
                    port,
                })
            }
            Ok(Ok(None)) => Err(K8sError::PodTimeout {
                name:         pod_name,
                timeout_secs: timeout.as_secs(),
            }),
            Ok(Err(source)) => Err(K8sError::WaitCondition { source }),
            Err(_elapsed) => {
                // Timeout — attempt cleanup.
                let _ = self.delete_pod(&pod_name, namespace).await;
                Err(K8sError::PodTimeout {
                    name:         pod_name,
                    timeout_secs: timeout.as_secs(),
                })
            }
        }
    }

    /// Delete a pod. Silently ignores `NotFound` errors (pod already gone).
    pub async fn delete_pod(&self, name: &str, namespace: &str) -> Result<(), K8sError> {
        let pods: Api<k8s_core::Pod> = Api::namespaced(self.client.clone(), namespace);

        match pods.delete(name, &DeleteParams::default()).await {
            Ok(_) => {
                info!(pod = %name, namespace = %namespace, "deleted pod");
                Ok(())
            }
            Err(kube::Error::Api(resp)) if resp.code == 404 => {
                debug!(pod = %name, "pod already deleted (NotFound)");
                Ok(())
            }
            Err(source) => Err(K8sError::KubeClient { source }),
        }
    }

    /// Get the current status of a pod.
    pub async fn get_pod_status(&self, name: &str, namespace: &str) -> Result<PodStatus, K8sError> {
        let pods: Api<k8s_core::Pod> = Api::namespaced(self.client.clone(), namespace);
        match pods.get(name).await {
            Ok(pod) => {
                let phase = pod
                    .status
                    .as_ref()
                    .and_then(|s| s.phase.clone())
                    .unwrap_or_else(|| "Unknown".to_string());
                let ready = phase == "Running";
                let ip = pod.status.as_ref().and_then(|s| s.pod_ip.clone());
                Ok(PodStatus {
                    name: name.to_string(),
                    namespace: namespace.to_string(),
                    phase,
                    ready,
                    ip,
                })
            }
            Err(source) => Err(K8sError::KubeClient { source }),
        }
    }

    /// Fetch container logs from a pod.
    pub async fn get_pod_logs(
        &self,
        name: &str,
        namespace: &str,
        tail_lines: Option<i64>,
    ) -> Result<String, K8sError> {
        let pods: Api<k8s_core::Pod> = Api::namespaced(self.client.clone(), namespace);
        let mut params = LogParams::default();
        if let Some(n) = tail_lines {
            params.tail_lines = Some(n);
        }
        pods.logs(name, &params)
            .await
            .map_err(|source| K8sError::KubeClient { source })
    }
}

// ── Public helpers ──────────────────────────────────────────────────

/// Generate a unique pod name: `{prefix}-{short_uuid}`.
pub fn generate_pod_name(prefix: &str) -> String {
    let short_id = &uuid::Uuid::new_v4().to_string()[..8];
    // K8s names must be lowercase, max 253 chars, [a-z0-9-].
    let sanitized = prefix
        .to_lowercase()
        .replace(|c: char| !c.is_ascii_alphanumeric() && c != '-', "-");
    let name = format!("{sanitized}-{short_id}");
    // Truncate to K8s max (253 chars).
    if name.len() > 253 {
        name[..253].to_string()
    } else {
        name
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_pod_name_format() {
        let name = generate_pod_name("my-server");
        assert!(name.starts_with("my-server-"), "got: {name}");
        // "my-server-" is 10 chars + 8 char uuid = 18
        assert_eq!(name.len(), "my-server-".len() + 8);
    }

    #[test]
    fn test_generate_pod_name_sanitization() {
        let name = generate_pod_name("My_Server.v2");
        // Uppercase and special chars replaced
        assert!(name.starts_with("my-server-v2-"), "got: {name}");
    }

    #[test]
    fn test_generate_pod_name_uniqueness() {
        let a = generate_pod_name("test");
        let b = generate_pod_name("test");
        assert_ne!(a, b, "pod names should be unique");
    }

    #[test]
    fn test_generate_pod_name_truncation() {
        let long_name = "a".repeat(300);
        let name = generate_pod_name(&long_name);
        assert!(name.len() <= 253, "name length: {}", name.len());
    }
}
