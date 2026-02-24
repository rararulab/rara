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

//! K8s Pod management for MCP servers.
//!
//! Creates ephemeral pods running MCP servers and connects via HTTP transport.
//! Each pod exposes an HTTP endpoint that the agent connects to using the
//! existing streamable HTTP client — K8s probes handle health checks, and Pod
//! isolation protects the host.

use std::collections::{BTreeMap, HashMap};

use k8s_openapi::api::core::v1::{
    Container, ContainerPort, EnvVar, HTTPGetAction, Pod, PodSpec, Probe,
};
use kube::{
    Api, Client,
    api::{DeleteParams, ObjectMeta, PostParams},
    runtime::wait::{await_condition, conditions::is_pod_running},
};
use snafu::Snafu;
use tracing::{debug, info};

/// Default namespace for MCP pods.
pub const DEFAULT_NAMESPACE: &str = "default";

/// Default container port the MCP server listens on.
pub const DEFAULT_PORT: u16 = 3000;

/// Timeout for waiting for a pod to become ready (seconds).
const POD_READY_TIMEOUT_SECS: u64 = 120;

// ── Error ───────────────────────────────────────────────────────────

/// Errors from K8s pod operations.
#[derive(Debug, Snafu)]
pub enum McpPodError {
    /// Failed to create or interact with the K8s API client.
    #[snafu(display("K8s client error: {source}"))]
    KubeClient { source: kube::Error },

    /// Error from the kube-runtime wait condition.
    #[snafu(display("K8s wait error: {source}"))]
    WaitCondition {
        source: kube::runtime::wait::Error,
    },

    /// Pod did not become ready within the timeout.
    #[snafu(display("Pod {name} failed to become ready within timeout"))]
    PodTimeout { name: String },

    /// Pod was created but has no IP assigned.
    #[snafu(display("Pod {name} has no IP assigned"))]
    NoPodIp { name: String },
}

// ── McpPodManager ───────────────────────────────────────────────────

/// Manages the lifecycle of ephemeral K8s pods running MCP servers.
///
/// Each pod runs a single container exposing an HTTP endpoint. After the pod
/// reaches `Running` + `Ready`, the caller connects to it via the existing
/// streamable HTTP client.
pub struct McpPodManager {
    client: Client,
}

impl McpPodManager {
    /// Create a new manager using the default cluster configuration.
    ///
    /// This uses the in-cluster config when running inside K8s, or the local
    /// kubeconfig (`~/.kube/config`) when running on a developer machine.
    pub async fn new() -> Result<Self, McpPodError> {
        let client = Client::try_default()
            .await
            .map_err(|source| McpPodError::KubeClient { source })?;
        Ok(Self { client })
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
        let pod_name = generate_pod_name(server_name);
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), namespace);

        let pod = build_pod_spec(&pod_name, image, port, env, labels);

        debug!(
            pod = %pod_name,
            namespace = %namespace,
            image = %image,
            "creating MCP pod"
        );

        pods.create(&PostParams::default(), &pod)
            .await
            .map_err(|source| McpPodError::KubeClient { source })?;

        // Wait for the pod to reach Running state.
        let running = await_condition(pods.clone(), &pod_name, is_pod_running());
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(POD_READY_TIMEOUT_SECS),
            running,
        )
        .await;

        match result {
            Ok(Ok(Some(pod_obj))) => {
                let pod_ip = pod_obj
                    .status
                    .as_ref()
                    .and_then(|s| s.pod_ip.as_deref())
                    .ok_or_else(|| McpPodError::NoPodIp {
                        name: pod_name.clone(),
                    })?
                    .to_string();

                info!(
                    pod = %pod_name,
                    ip = %pod_ip,
                    port = port,
                    "MCP pod is running"
                );

                Ok((pod_name, pod_ip, port))
            }
            Ok(Ok(None)) => Err(McpPodError::PodTimeout {
                name: pod_name,
            }),
            Ok(Err(source)) => Err(McpPodError::WaitCondition { source }),
            Err(_elapsed) => {
                // Timeout — attempt cleanup.
                let _ = self.delete_mcp_pod(&pod_name, namespace).await;
                Err(McpPodError::PodTimeout {
                    name: pod_name,
                })
            }
        }
    }

    /// Delete an MCP server pod.
    ///
    /// Silently ignores `NotFound` errors (pod already deleted).
    pub async fn delete_mcp_pod(
        &self,
        pod_name: &str,
        namespace: &str,
    ) -> Result<(), McpPodError> {
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), namespace);

        match pods.delete(pod_name, &DeleteParams::default()).await {
            Ok(_) => {
                info!(pod = %pod_name, namespace = %namespace, "deleted MCP pod");
                Ok(())
            }
            Err(kube::Error::Api(resp)) if resp.code == 404 => {
                debug!(pod = %pod_name, "MCP pod already deleted (NotFound)");
                Ok(())
            }
            Err(source) => Err(McpPodError::KubeClient { source }),
        }
    }

    /// Check if a pod is still running.
    pub async fn is_pod_running(
        &self,
        pod_name: &str,
        namespace: &str,
    ) -> Result<bool, McpPodError> {
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), namespace);
        match pods.get(pod_name).await {
            Ok(pod) => {
                let phase = pod
                    .status
                    .as_ref()
                    .and_then(|s| s.phase.as_deref())
                    .unwrap_or("Unknown");
                Ok(phase == "Running")
            }
            Err(kube::Error::Api(resp)) if resp.code == 404 => Ok(false),
            Err(source) => Err(McpPodError::KubeClient { source }),
        }
    }
}

// ── Private helpers ─────────────────────────────────────────────────

/// Generate a unique pod name: `mcp-{server_name}-{short_uuid}`.
fn generate_pod_name(server_name: &str) -> String {
    let short_id = &uuid::Uuid::new_v4().to_string()[..8];
    // K8s names must be lowercase, max 253 chars, [a-z0-9-].
    let sanitized = server_name
        .to_lowercase()
        .replace(|c: char| !c.is_ascii_alphanumeric() && c != '-', "-");
    let name = format!("mcp-{sanitized}-{short_id}");
    // Truncate to K8s max (253 chars).
    if name.len() > 253 {
        name[..253].to_string()
    } else {
        name
    }
}

/// Build a K8s Pod spec for an MCP server container.
fn build_pod_spec(
    pod_name: &str,
    image: &str,
    port: u16,
    env: &HashMap<String, String>,
    labels: Option<&HashMap<String, String>>,
) -> Pod {
    // Build labels: always include our management labels.
    let mut pod_labels = BTreeMap::new();
    pod_labels.insert("app.kubernetes.io/managed-by".to_string(), "rara-mcp".to_string());
    pod_labels.insert("rara.dev/component".to_string(), "mcp-server".to_string());
    pod_labels.insert("rara.dev/pod-name".to_string(), pod_name.to_string());
    if let Some(extra) = labels {
        for (k, v) in extra {
            pod_labels.insert(k.clone(), v.clone());
        }
    }

    // Build environment variables for the container.
    let env_vars: Vec<EnvVar> = env
        .iter()
        .map(|(k, v)| EnvVar {
            name:       k.clone(),
            value:      Some(v.clone()),
            value_from: None,
        })
        .collect();

    let i32_port = i32::from(port);

    // HTTP probe for liveness and readiness.
    let http_probe = Probe {
        http_get:              Some(HTTPGetAction {
            path:         Some("/".to_string()),
            port:         k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(i32_port),
            scheme:       Some("HTTP".to_string()),
            host:         None,
            http_headers: None,
        }),
        initial_delay_seconds: Some(5),
        period_seconds:        Some(10),
        timeout_seconds:       Some(5),
        failure_threshold:     Some(3),
        success_threshold:     Some(1),
        ..Default::default()
    };

    Pod {
        metadata: ObjectMeta {
            name:   Some(pod_name.to_string()),
            labels: Some(pod_labels),
            ..Default::default()
        },
        spec:     Some(PodSpec {
            restart_policy: Some("Never".to_string()),
            containers:     vec![Container {
                name:            "mcp-server".to_string(),
                image:           Some(image.to_string()),
                ports:           Some(vec![ContainerPort {
                    container_port: i32_port,
                    protocol:       Some("TCP".to_string()),
                    name:           Some("http".to_string()),
                    ..Default::default()
                }]),
                env:             Some(env_vars),
                liveness_probe:  Some(http_probe.clone()),
                readiness_probe: Some(http_probe),
                ..Default::default()
            }],
            ..Default::default()
        }),
        status:   None,
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_pod_name_format() {
        let name = generate_pod_name("my-server");
        assert!(name.starts_with("mcp-my-server-"), "got: {name}");
        // 4 + 1 + 9 + 1 + 8 = 23 chars
        assert_eq!(name.len(), "mcp-my-server-".len() + 8);
    }

    #[test]
    fn test_generate_pod_name_sanitization() {
        let name = generate_pod_name("My_Server.v2");
        // Uppercase and special chars replaced
        assert!(name.starts_with("mcp-my-server-v2-"), "got: {name}");
    }

    #[test]
    fn test_generate_pod_name_uniqueness() {
        let a = generate_pod_name("test");
        let b = generate_pod_name("test");
        assert_ne!(a, b, "pod names should be unique");
    }

    #[test]
    fn test_build_pod_spec_basic() {
        let env = HashMap::from([("KEY".to_string(), "value".to_string())]);
        let pod = build_pod_spec("mcp-test-12345678", "my-image:latest", 3000, &env, None);

        // Check metadata.
        let meta = &pod.metadata;
        assert_eq!(meta.name.as_deref(), Some("mcp-test-12345678"));
        let labels = meta.labels.as_ref().unwrap();
        assert_eq!(labels.get("app.kubernetes.io/managed-by").unwrap(), "rara-mcp");
        assert_eq!(labels.get("rara.dev/component").unwrap(), "mcp-server");

        // Check container.
        let spec = pod.spec.as_ref().unwrap();
        assert_eq!(spec.restart_policy.as_deref(), Some("Never"));
        assert_eq!(spec.containers.len(), 1);

        let container = &spec.containers[0];
        assert_eq!(container.name, "mcp-server");
        assert_eq!(container.image.as_deref(), Some("my-image:latest"));

        // Check port.
        let ports = container.ports.as_ref().unwrap();
        assert_eq!(ports.len(), 1);
        assert_eq!(ports[0].container_port, 3000);

        // Check env.
        let env_vars = container.env.as_ref().unwrap();
        assert_eq!(env_vars.len(), 1);
        assert_eq!(env_vars[0].name, "KEY");
        assert_eq!(env_vars[0].value.as_deref(), Some("value"));

        // Check probes.
        assert!(container.liveness_probe.is_some());
        assert!(container.readiness_probe.is_some());
    }

    #[test]
    fn test_build_pod_spec_with_labels() {
        let env = HashMap::new();
        let extra_labels = HashMap::from([
            ("team".to_string(), "platform".to_string()),
            ("version".to_string(), "v1".to_string()),
        ]);
        let pod = build_pod_spec(
            "mcp-labeled-12345678",
            "my-image:latest",
            8080,
            &env,
            Some(&extra_labels),
        );

        let labels = pod.metadata.labels.as_ref().unwrap();
        // Default labels still present.
        assert_eq!(labels.get("app.kubernetes.io/managed-by").unwrap(), "rara-mcp");
        // Extra labels merged.
        assert_eq!(labels.get("team").unwrap(), "platform");
        assert_eq!(labels.get("version").unwrap(), "v1");
    }

    #[test]
    fn test_build_pod_spec_custom_port() {
        let env = HashMap::new();
        let pod = build_pod_spec("mcp-custom-12345678", "img:v1", 8080, &env, None);

        let container = &pod.spec.as_ref().unwrap().containers[0];
        let ports = container.ports.as_ref().unwrap();
        assert_eq!(ports[0].container_port, 8080);

        // Probes should also target port 8080.
        let probe = container.liveness_probe.as_ref().unwrap();
        let http_get = probe.http_get.as_ref().unwrap();
        assert_eq!(
            http_get.port,
            k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(8080)
        );
    }

    #[test]
    fn test_generate_pod_name_truncation() {
        let long_name = "a".repeat(300);
        let name = generate_pod_name(&long_name);
        assert!(name.len() <= 253, "name length: {}", name.len());
    }
}
