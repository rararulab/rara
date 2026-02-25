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

//! Generic Kubernetes Pod management.
//!
//! Provides [`PodManager`] for creating, deleting, and inspecting ephemeral
//! pods. This crate is transport-agnostic — it knows nothing about MCP,
//! agents, or any particular use case. Higher-level crates (e.g. `rara-mcp`,
//! `tool-core`) wrap `PodManager` with domain-specific defaults.
//!
//! Callers construct a [`k8s_types::Pod`] directly using the re-exported
//! `k8s-openapi` types and pass it to [`PodManager::create_pod`].

pub mod error;
pub mod manager;
pub mod types;

pub use error::K8sError;
pub use manager::{PodManager, generate_pod_name};
pub use types::*;

/// Re-export commonly used `k8s-openapi` types so callers don't need to
/// depend on `k8s-openapi` directly.
pub mod k8s_types {
    pub use k8s_openapi::api::core::v1::{
        Container, ContainerPort, EnvVar, HTTPGetAction, Pod, PodSpec, Probe,
        ResourceRequirements,
    };
    pub use k8s_openapi::apimachinery::pkg::{
        api::resource::Quantity, util::intstr::IntOrString,
    };
    pub use kube::api::ObjectMeta;
}
