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

//! Mem0-specific K8s Pod management.
//!
//! Creates ephemeral mem0 pods on demand when an OpenAI API key becomes
//! available at runtime. The pod runs `mem0/mem0-api-server` with ChromaDB
//! as the vector backend.

use std::collections::BTreeMap;
use std::time::Duration;

use rara_k8s::k8s_types::*;

/// Default namespace for mem0 pods.
pub const DEFAULT_NAMESPACE: &str = "default";

/// Default container port the mem0 server listens on.
pub const DEFAULT_PORT: u16 = 8000;

/// Default Docker image for mem0 API server.
pub const DEFAULT_IMAGE: &str = "mem0/mem0-api-server:latest";

/// Re-export the underlying error so callers do not need to depend on
/// `rara-k8s` directly.
pub use rara_k8s::K8sError as Mem0PodError;

// -- Mem0PodManager ---------------------------------------------------------

/// Manages the lifecycle of ephemeral K8s pods running the mem0 API server.
///
/// Delegates to [`rara_k8s::PodManager`] and applies mem0-specific defaults
/// (management labels, startup command, health probes).
pub struct Mem0PodManager {
    inner: rara_k8s::PodManager,
}

impl Mem0PodManager {
    /// Create a new manager using the default cluster configuration.
    pub async fn new() -> Result<Self, Mem0PodError> {
        Ok(Self {
            inner: rara_k8s::PodManager::new().await?,
        })
    }

    /// Create a mem0 pod with the given OpenAI API key and ChromaDB connection.
    ///
    /// The pod runs a startup command that:
    /// 1. `pip install chromadb`
    /// 2. Patches mem0's config.yaml to use ChromaDB (instead of pgvector) and
    ///    removes neo4j graph_store
    /// 3. Starts uvicorn
    ///
    /// Returns `(pod_name, pod_ip, port)`.
    pub async fn create_mem0_pod(
        &self,
        openai_api_key: &str,
        chroma_host: &str,
        chroma_port: u16,
        image: &str,
        namespace: &str,
    ) -> Result<(String, String, u16), Mem0PodError> {
        let pod_name = rara_k8s::generate_pod_name("rara-mem0");

        let mut labels = BTreeMap::new();
        labels.insert("app.kubernetes.io/managed-by".into(), "rara".into());
        labels.insert("rara.dev/component".into(), "mem0".into());
        labels.insert("app".into(), "mem0".into());

        let env_vars = vec![
            EnvVar {
                name: "OPENAI_API_KEY".into(),
                value: Some(openai_api_key.into()),
                value_from: None,
            },
            EnvVar {
                name: "CHROMA_HOST".into(),
                value: Some(chroma_host.into()),
                value_from: None,
            },
            EnvVar {
                name: "CHROMA_PORT".into(),
                value: Some(chroma_port.to_string()),
                value_from: None,
            },
            EnvVar {
                name: "HISTORY_DB_PATH".into(),
                value: Some("/tmp/mem0_history.db".into()),
                value_from: None,
            },
        ];

        // Startup command: install chromadb, patch config, start server.
        // This mirrors the Helm deployment's init container approach.
        let command = vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            format!(
                r#"pip install chromadb && \
python3 -c "
import yaml, os
config_path = '/app/config.yaml'
if os.path.exists(config_path):
    with open(config_path) as f:
        config = yaml.safe_load(f) or {{}}
else:
    config = {{}}
config['vector_store'] = {{'provider': 'chroma', 'config': {{'collection_name': 'mem0', 'host': os.environ['CHROMA_HOST'], 'port': int(os.environ['CHROMA_PORT'])}}}}
config.pop('graph_store', None)
with open(config_path, 'w') as f:
    yaml.dump(config, f)
print('Config patched:', config)
" && \
exec uvicorn main:app --host 0.0.0.0 --port 8000 --workers 1"#
            ),
        ];

        let i32_port = i32::from(DEFAULT_PORT);
        let probe = Probe {
            http_get: Some(HTTPGetAction {
                path: Some("/".into()),
                port: IntOrString::Int(i32_port),
                scheme: Some("HTTP".into()),
                host: None,
                http_headers: None,
            }),
            initial_delay_seconds: Some(60),
            period_seconds: Some(15),
            timeout_seconds: Some(5),
            failure_threshold: Some(5),
            success_threshold: Some(1),
            ..Default::default()
        };

        let pod = Pod {
            metadata: ObjectMeta {
                name: Some(pod_name.clone()),
                labels: Some(labels),
                ..Default::default()
            },
            spec: Some(PodSpec {
                restart_policy: Some("Never".into()),
                containers: vec![Container {
                    name: "mem0".into(),
                    image: Some(image.into()),
                    command: Some(command),
                    ports: Some(vec![ContainerPort {
                        container_port: i32_port,
                        protocol: Some("TCP".into()),
                        ..Default::default()
                    }]),
                    env: Some(env_vars),
                    liveness_probe: Some(probe.clone()),
                    readiness_probe: Some(probe),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            status: None,
        };

        let handle = self
            .inner
            .create_pod(pod, namespace, Duration::from_secs(300))
            .await?;
        let ip = handle
            .ip
            .ok_or_else(|| rara_k8s::K8sError::NoPodIp {
                name: handle.name.clone(),
            })?;
        Ok((handle.name, ip, handle.port.unwrap_or(DEFAULT_PORT)))
    }

    /// Delete a mem0 pod.
    ///
    /// Silently ignores `NotFound` errors (pod already deleted).
    pub async fn delete_mem0_pod(
        &self,
        pod_name: &str,
        namespace: &str,
    ) -> Result<(), Mem0PodError> {
        self.inner.delete_pod(pod_name, namespace).await
    }

    /// Check if a pod is still running.
    pub async fn is_pod_running(
        &self,
        pod_name: &str,
        namespace: &str,
    ) -> Result<bool, Mem0PodError> {
        let status = self.inner.get_pod_status(pod_name, namespace).await?;
        Ok(status.ready)
    }
}

// -- Tests ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_constants() {
        assert_eq!(DEFAULT_NAMESPACE, "default");
        assert_eq!(DEFAULT_PORT, 8000);
        assert_eq!(DEFAULT_IMAGE, "mem0/mem0-api-server:latest");
    }
}
