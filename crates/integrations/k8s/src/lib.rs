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

pub mod error;
pub mod manager;
pub mod types;

pub use error::K8sError;
pub use manager::PodManager;
pub use types::*;
