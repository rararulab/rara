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

//! Pod configuration and result types.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Specification for creating a new pod.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodSpec {
    /// Prefix used to generate a unique pod name.
    pub name_prefix: String,
    /// Container image to run.
    pub image: String,
    /// Kubernetes namespace. Defaults to `"default"`.
    #[serde(default = "default_namespace")]
    pub namespace: String,
    /// Optional container port to expose.
    pub port: Option<u16>,
    /// Override the container entrypoint.
    pub command: Option<Vec<String>>,
    /// Arguments passed to the container.
    pub args: Option<Vec<String>>,
    /// Environment variables for the container.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Extra labels applied to the pod metadata.
    #[serde(default)]
    pub labels: HashMap<String, String>,
    /// Resource requests/limits.
    pub resources: Option<ResourceSpec>,
    /// HTTP probe configuration for liveness/readiness.
    pub probe: Option<ProbeSpec>,
    /// Pod restart policy.
    #[serde(default)]
    pub restart_policy: RestartPolicy,
    /// Timeout (seconds) to wait for the pod to reach Running state.
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

fn default_namespace() -> String {
    "default".to_string()
}

fn default_timeout() -> u64 {
    120
}

/// Handle returned after a pod is successfully created.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodHandle {
    /// The generated pod name.
    pub name: String,
    /// The namespace the pod was created in.
    pub namespace: String,
    /// The pod's cluster IP (if assigned).
    pub ip: Option<String>,
    /// The container port (if configured).
    pub port: Option<u16>,
}

/// Current status of a pod.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodStatus {
    pub name: String,
    pub namespace: String,
    /// Pod phase (e.g. "Running", "Pending", "Failed").
    pub phase: String,
    /// Whether the pod is considered ready.
    pub ready: bool,
    /// The pod's cluster IP (if assigned).
    pub ip: Option<String>,
}

/// Output from executing a command inside a pod.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
}

/// Resource requests and limits for a container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceSpec {
    pub cpu_limit: Option<String>,
    pub memory_limit: Option<String>,
    pub cpu_request: Option<String>,
    pub memory_request: Option<String>,
}

/// HTTP probe configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeSpec {
    /// HTTP path to probe (e.g. `"/healthz"`).
    pub http_path: Option<String>,
    /// Port to probe.
    pub port: u16,
    /// Seconds before starting probes after container start.
    pub initial_delay_secs: Option<i32>,
    /// Seconds between probes.
    pub period_secs: Option<i32>,
}

/// Restart policy for the pod.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RestartPolicy {
    #[default]
    Never,
    OnFailure,
    Always,
}
