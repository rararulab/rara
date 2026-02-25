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

//! Pod result and status types.

use serde::{Deserialize, Serialize};

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
