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

//! Core Pod lifecycle manager.
//!
//! [`PodManager`] creates, deletes, and inspects ephemeral K8s pods.
//! It is intentionally generic — no MCP or agent logic here.

use std::collections::BTreeMap;

use k8s_openapi::api::core::v1 as k8s_core;
use kube::{
    Api, Client,
    api::{DeleteParams, LogParams, PostParams},
    runtime::wait::{await_condition, conditions::is_pod_running},
};
use tracing::{debug, info};

use crate::error::K8sError;
use crate::types::{PodHandle, PodSpec, PodStatus, ProbeSpec, ResourceSpec, RestartPolicy};

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
    pub fn with_client(client: Client) -> Self {
        Self { client }
    }

    /// Create a pod and wait until it reaches the `Running` phase.
    pub async fn create_pod(&self, spec: PodSpec) -> Result<PodHandle, K8sError> {
        let pod_name = generate_pod_name(&spec.name_prefix);
        let pods: Api<k8s_core::Pod> = Api::namespaced(self.client.clone(), &spec.namespace);

        let pod = build_k8s_pod(&pod_name, &spec);

        debug!(
            pod = %pod_name,
            namespace = %spec.namespace,
            image = %spec.image,
            "creating pod"
        );

        pods.create(&PostParams::default(), &pod)
            .await
            .map_err(|source| K8sError::KubeClient { source })?;

        // Wait for the pod to reach Running state.
        let running = await_condition(pods.clone(), &pod_name, is_pod_running());
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(spec.timeout_secs),
            running,
        )
        .await;

        match result {
            Ok(Ok(Some(pod_obj))) => {
                let ip = pod_obj
                    .status
                    .as_ref()
                    .and_then(|s| s.pod_ip.clone());

                info!(pod = %pod_name, ip = ?ip, "pod is running");

                Ok(PodHandle {
                    name: pod_name,
                    namespace: spec.namespace,
                    ip,
                    port: spec.port,
                })
            }
            Ok(Ok(None)) => Err(K8sError::PodTimeout {
                name: pod_name,
                timeout_secs: spec.timeout_secs,
            }),
            Ok(Err(source)) => Err(K8sError::WaitCondition { source }),
            Err(_elapsed) => {
                // Timeout — attempt cleanup.
                let _ = self.delete_pod(&pod_name, &spec.namespace).await;
                Err(K8sError::PodTimeout {
                    name: pod_name,
                    timeout_secs: spec.timeout_secs,
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
    pub async fn get_pod_status(
        &self,
        name: &str,
        namespace: &str,
    ) -> Result<PodStatus, K8sError> {
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

// ── Private helpers ─────────────────────────────────────────────────

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

/// Build a K8s [`Pod`](k8s_core::Pod) object from our [`PodSpec`].
fn build_k8s_pod(pod_name: &str, spec: &PodSpec) -> k8s_core::Pod {
    // Build labels: always include our management label.
    let mut pod_labels = BTreeMap::new();
    pod_labels.insert(
        "app.kubernetes.io/managed-by".to_string(),
        "rara".to_string(),
    );
    for (k, v) in &spec.labels {
        pod_labels.insert(k.clone(), v.clone());
    }

    // Build environment variables for the container.
    let env_vars: Vec<k8s_core::EnvVar> = spec
        .env
        .iter()
        .map(|(k, v)| k8s_core::EnvVar {
            name: k.clone(),
            value: Some(v.clone()),
            value_from: None,
        })
        .collect();

    // Build probe if configured.
    let probe = spec.probe.as_ref().map(build_probe);

    // Build resource requirements if configured.
    let resources = spec.resources.as_ref().map(build_resources);

    let restart = match spec.restart_policy {
        RestartPolicy::Never => "Never",
        RestartPolicy::OnFailure => "OnFailure",
        RestartPolicy::Always => "Always",
    };

    k8s_core::Pod {
        metadata: kube::api::ObjectMeta {
            name: Some(pod_name.to_string()),
            labels: Some(pod_labels),
            ..Default::default()
        },
        spec: Some(k8s_core::PodSpec {
            restart_policy: Some(restart.to_string()),
            containers: vec![k8s_core::Container {
                name: "main".to_string(),
                image: Some(spec.image.clone()),
                command: spec.command.clone(),
                args: spec.args.clone(),
                ports: spec.port.map(|p| {
                    vec![k8s_core::ContainerPort {
                        container_port: i32::from(p),
                        protocol: Some("TCP".to_string()),
                        ..Default::default()
                    }]
                }),
                env: if env_vars.is_empty() {
                    None
                } else {
                    Some(env_vars)
                },
                liveness_probe: probe.clone(),
                readiness_probe: probe,
                resources,
                ..Default::default()
            }],
            ..Default::default()
        }),
        status: None,
    }
}

fn build_probe(p: &ProbeSpec) -> k8s_core::Probe {
    let i32_port = i32::from(p.port);
    k8s_core::Probe {
        http_get: p.http_path.as_ref().map(|path| k8s_core::HTTPGetAction {
            path: Some(path.clone()),
            port: k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(i32_port),
            scheme: Some("HTTP".to_string()),
            host: None,
            http_headers: None,
        }),
        initial_delay_seconds: p.initial_delay_secs,
        period_seconds: p.period_secs,
        timeout_seconds: Some(5),
        failure_threshold: Some(3),
        success_threshold: Some(1),
        ..Default::default()
    }
}

fn build_resources(r: &ResourceSpec) -> k8s_core::ResourceRequirements {
    use k8s_openapi::apimachinery::pkg::api::resource::Quantity;

    let mut limits = BTreeMap::new();
    let mut requests = BTreeMap::new();

    if let Some(ref cpu) = r.cpu_limit {
        limits.insert("cpu".to_string(), Quantity(cpu.clone()));
    }
    if let Some(ref mem) = r.memory_limit {
        limits.insert("memory".to_string(), Quantity(mem.clone()));
    }
    if let Some(ref cpu) = r.cpu_request {
        requests.insert("cpu".to_string(), Quantity(cpu.clone()));
    }
    if let Some(ref mem) = r.memory_request {
        requests.insert("memory".to_string(), Quantity(mem.clone()));
    }

    k8s_core::ResourceRequirements {
        limits: if limits.is_empty() {
            None
        } else {
            Some(limits)
        },
        requests: if requests.is_empty() {
            None
        } else {
            Some(requests)
        },
        ..Default::default()
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

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

    #[test]
    fn test_build_k8s_pod_basic() {
        let spec = PodSpec {
            name_prefix: "test".to_string(),
            image: "my-image:latest".to_string(),
            namespace: "default".to_string(),
            port: Some(3000),
            command: None,
            args: None,
            env: HashMap::from([("KEY".to_string(), "value".to_string())]),
            labels: HashMap::new(),
            resources: None,
            probe: None,
            restart_policy: RestartPolicy::Never,
            timeout_secs: 120,
        };

        let pod = build_k8s_pod("test-12345678", &spec);

        // Check metadata.
        let meta = &pod.metadata;
        assert_eq!(meta.name.as_deref(), Some("test-12345678"));
        let labels = meta.labels.as_ref().unwrap();
        assert_eq!(
            labels.get("app.kubernetes.io/managed-by").unwrap(),
            "rara"
        );

        // Check container.
        let pod_spec = pod.spec.as_ref().unwrap();
        assert_eq!(pod_spec.restart_policy.as_deref(), Some("Never"));
        assert_eq!(pod_spec.containers.len(), 1);

        let container = &pod_spec.containers[0];
        assert_eq!(container.name, "main");
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

        // No probes configured.
        assert!(container.liveness_probe.is_none());
        assert!(container.readiness_probe.is_none());
    }

    #[test]
    fn test_build_k8s_pod_with_probe() {
        let spec = PodSpec {
            name_prefix: "test".to_string(),
            image: "my-image:latest".to_string(),
            namespace: "default".to_string(),
            port: Some(8080),
            command: None,
            args: None,
            env: HashMap::new(),
            labels: HashMap::new(),
            resources: None,
            probe: Some(ProbeSpec {
                http_path: Some("/healthz".to_string()),
                port: 8080,
                initial_delay_secs: Some(5),
                period_secs: Some(10),
            }),
            restart_policy: RestartPolicy::Never,
            timeout_secs: 120,
        };

        let pod = build_k8s_pod("test-probed-12345678", &spec);
        let container = &pod.spec.as_ref().unwrap().containers[0];

        assert!(container.liveness_probe.is_some());
        assert!(container.readiness_probe.is_some());

        let probe = container.liveness_probe.as_ref().unwrap();
        let http_get = probe.http_get.as_ref().unwrap();
        assert_eq!(http_get.path.as_deref(), Some("/healthz"));
        assert_eq!(
            http_get.port,
            k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(8080)
        );
    }

    #[test]
    fn test_build_k8s_pod_with_labels() {
        let spec = PodSpec {
            name_prefix: "test".to_string(),
            image: "my-image:latest".to_string(),
            namespace: "default".to_string(),
            port: None,
            command: None,
            args: None,
            env: HashMap::new(),
            labels: HashMap::from([
                ("team".to_string(), "platform".to_string()),
                ("version".to_string(), "v1".to_string()),
            ]),
            resources: None,
            probe: None,
            restart_policy: RestartPolicy::Never,
            timeout_secs: 120,
        };

        let pod = build_k8s_pod("test-labeled-12345678", &spec);
        let labels = pod.metadata.labels.as_ref().unwrap();

        // Default label still present.
        assert_eq!(
            labels.get("app.kubernetes.io/managed-by").unwrap(),
            "rara"
        );
        // Extra labels merged.
        assert_eq!(labels.get("team").unwrap(), "platform");
        assert_eq!(labels.get("version").unwrap(), "v1");
    }

    #[test]
    fn test_build_k8s_pod_with_resources() {
        let spec = PodSpec {
            name_prefix: "test".to_string(),
            image: "my-image:latest".to_string(),
            namespace: "default".to_string(),
            port: None,
            command: None,
            args: None,
            env: HashMap::new(),
            labels: HashMap::new(),
            resources: Some(ResourceSpec {
                cpu_limit: Some("500m".to_string()),
                memory_limit: Some("256Mi".to_string()),
                cpu_request: Some("100m".to_string()),
                memory_request: Some("64Mi".to_string()),
            }),
            probe: None,
            restart_policy: RestartPolicy::Never,
            timeout_secs: 120,
        };

        let pod = build_k8s_pod("test-resources-12345678", &spec);
        let container = &pod.spec.as_ref().unwrap().containers[0];
        let resources = container.resources.as_ref().unwrap();

        let limits = resources.limits.as_ref().unwrap();
        assert_eq!(limits.get("cpu").unwrap().0, "500m");
        assert_eq!(limits.get("memory").unwrap().0, "256Mi");

        let requests = resources.requests.as_ref().unwrap();
        assert_eq!(requests.get("cpu").unwrap().0, "100m");
        assert_eq!(requests.get("memory").unwrap().0, "64Mi");
    }

    #[test]
    fn test_build_k8s_pod_restart_policies() {
        for (policy, expected) in [
            (RestartPolicy::Never, "Never"),
            (RestartPolicy::OnFailure, "OnFailure"),
            (RestartPolicy::Always, "Always"),
        ] {
            let spec = PodSpec {
                name_prefix: "test".to_string(),
                image: "img:v1".to_string(),
                namespace: "default".to_string(),
                port: None,
                command: None,
                args: None,
                env: HashMap::new(),
                labels: HashMap::new(),
                resources: None,
                probe: None,
                restart_policy: policy,
                timeout_secs: 120,
            };
            let pod = build_k8s_pod("test-restart-12345678", &spec);
            assert_eq!(
                pod.spec.as_ref().unwrap().restart_policy.as_deref(),
                Some(expected)
            );
        }
    }

    #[test]
    fn test_build_k8s_pod_with_command_and_args() {
        let spec = PodSpec {
            name_prefix: "test".to_string(),
            image: "python:3.12".to_string(),
            namespace: "default".to_string(),
            port: None,
            command: Some(vec!["python".to_string()]),
            args: Some(vec!["-c".to_string(), "print('hello')".to_string()]),
            env: HashMap::new(),
            labels: HashMap::new(),
            resources: None,
            probe: None,
            restart_policy: RestartPolicy::Never,
            timeout_secs: 120,
        };

        let pod = build_k8s_pod("test-cmd-12345678", &spec);
        let container = &pod.spec.as_ref().unwrap().containers[0];

        assert_eq!(
            container.command.as_deref(),
            Some(["python".to_string()].as_slice())
        );
        assert_eq!(
            container.args.as_deref(),
            Some(["-c".to_string(), "print('hello')".to_string()].as_slice())
        );
    }
}
