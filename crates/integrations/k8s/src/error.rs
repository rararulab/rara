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

//! Error types for Kubernetes operations.

use snafu::Snafu;

/// Errors from K8s pod operations.
#[derive(Debug, Snafu)]
pub enum K8sError {
    /// Failed to create or interact with the K8s API client.
    #[snafu(display("K8s client error: {source}"))]
    KubeClient { source: kube::Error },

    /// Error from the kube-runtime wait condition.
    #[snafu(display("K8s wait error: {source}"))]
    WaitCondition {
        source: kube::runtime::wait::Error,
    },

    /// Pod did not become ready within the timeout.
    #[snafu(display("Pod {name} failed to become ready within {timeout_secs}s"))]
    PodTimeout { name: String, timeout_secs: u64 },

    /// Pod was created but has no IP assigned.
    #[snafu(display("Pod {name} has no IP assigned"))]
    NoPodIp { name: String },

    /// A command executed inside the pod failed.
    #[snafu(display("Pod exec error: {message}"))]
    ExecFailed { message: String },
}
