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

//! PodTool — lets agents dynamically manage K8s pods.
//!
//! Wraps [`rara_k8s::PodManager`] behind the [`AgentTool`] trait so agents
//! can create, delete, inspect, and read logs from ephemeral pods.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use crate::AgentTool;
use rara_k8s::k8s_types::*;

/// Agent-callable tool for managing Kubernetes pods.
pub struct PodTool {
    manager: Arc<rara_k8s::PodManager>,
}

/// Internal representation of the action dispatched by the agent.
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum PodAction {
    Create {
        image: String,
        #[serde(default = "default_namespace")]
        namespace: String,
        port: Option<u16>,
        command: Option<Vec<String>>,
        args: Option<Vec<String>>,
        #[serde(default)]
        env: HashMap<String, String>,
        #[serde(default)]
        labels: HashMap<String, String>,
        #[serde(default = "default_name_prefix")]
        name_prefix: String,
    },
    Delete {
        pod_name: String,
        #[serde(default = "default_namespace")]
        namespace: String,
    },
    Status {
        pod_name: String,
        #[serde(default = "default_namespace")]
        namespace: String,
    },
    Logs {
        pod_name: String,
        #[serde(default = "default_namespace")]
        namespace: String,
        tail_lines: Option<i64>,
    },
}

fn default_namespace() -> String {
    "default".to_string()
}

fn default_name_prefix() -> String {
    "rara-pod".to_string()
}

impl PodTool {
    pub fn new(manager: Arc<rara_k8s::PodManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl AgentTool for PodTool {
    fn name(&self) -> &str {
        "pod"
    }

    fn description(&self) -> &str {
        "Manage Kubernetes pods. Actions: create, delete, status, logs. \
         Use for running isolated workloads in the cluster."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["action"],
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create", "delete", "status", "logs"],
                    "description": "The operation to perform"
                },
                "image": {
                    "type": "string",
                    "description": "Container image (required for create)"
                },
                "name_prefix": {
                    "type": "string",
                    "description": "Pod name prefix (for create)",
                    "default": "rara-pod"
                },
                "namespace": {
                    "type": "string",
                    "description": "K8s namespace",
                    "default": "default"
                },
                "port": {
                    "type": "integer",
                    "description": "Container port to expose (for create)"
                },
                "command": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Override container entrypoint (for create)"
                },
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Container arguments (for create)"
                },
                "env": {
                    "type": "object",
                    "description": "Environment variables (for create)"
                },
                "labels": {
                    "type": "object",
                    "description": "Extra pod labels (for create)"
                },
                "pod_name": {
                    "type": "string",
                    "description": "Pod name (required for delete/status/logs)"
                },
                "tail_lines": {
                    "type": "integer",
                    "description": "Number of log lines to return (for logs)"
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let action: PodAction = serde_json::from_value(params)?;
        match action {
            PodAction::Create {
                image,
                namespace,
                port,
                command,
                args,
                env,
                labels,
                name_prefix,
            } => {
                let pod_name = rara_k8s::generate_pod_name(&name_prefix);

                let mut pod_labels = BTreeMap::new();
                pod_labels
                    .insert("app.kubernetes.io/managed-by".into(), "rara".into());
                for (k, v) in &labels {
                    pod_labels.insert(k.clone(), v.clone());
                }

                let env_vars: Vec<EnvVar> = env
                    .iter()
                    .map(|(k, v)| EnvVar {
                        name: k.clone(),
                        value: Some(v.clone()),
                        value_from: None,
                    })
                    .collect();

                let pod = Pod {
                    metadata: ObjectMeta {
                        name: Some(pod_name),
                        labels: Some(pod_labels),
                        ..Default::default()
                    },
                    spec: Some(PodSpec {
                        restart_policy: Some("Never".into()),
                        containers: vec![Container {
                            name: "main".into(),
                            image: Some(image),
                            command,
                            args,
                            ports: port.map(|p| {
                                vec![ContainerPort {
                                    container_port: i32::from(p),
                                    protocol: Some("TCP".into()),
                                    ..Default::default()
                                }]
                            }),
                            env: if env_vars.is_empty() {
                                None
                            } else {
                                Some(env_vars)
                            },
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                    status: None,
                };

                let handle = self
                    .manager
                    .create_pod(pod, &namespace, Duration::from_secs(120))
                    .await?;
                Ok(serde_json::to_value(&handle)?)
            }
            PodAction::Delete {
                pod_name,
                namespace,
            } => {
                self.manager.delete_pod(&pod_name, &namespace).await?;
                Ok(serde_json::json!({"deleted": pod_name}))
            }
            PodAction::Status {
                pod_name,
                namespace,
            } => {
                let status = self
                    .manager
                    .get_pod_status(&pod_name, &namespace)
                    .await?;
                Ok(serde_json::to_value(&status)?)
            }
            PodAction::Logs {
                pod_name,
                namespace,
                tail_lines,
            } => {
                let logs = self
                    .manager
                    .get_pod_logs(&pod_name, &namespace, tail_lines)
                    .await?;
                Ok(serde_json::json!({"logs": logs}))
            }
        }
    }
}
