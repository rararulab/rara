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

//! Core trait for agent-callable tools.
//!
//! This crate defines the [`AgentTool`] trait and [`AgentToolRef`] type alias
//! used by the agent runtime and tool implementations across the workspace.

use std::sync::Arc;

use async_trait::async_trait;

/// Reference-counted handle to an agent tool.
pub type AgentToolRef = Arc<dyn AgentTool>;

pub mod core_primitives;
pub mod domain_primitives;

/// Agent-callable tool.
#[async_trait]
pub trait AgentTool: Send + Sync {
    /// Unique name of the tool.
    fn name(&self) -> &str;

    /// Human-readable description of the tool's purpose.
    fn description(&self) -> &str;

    /// JSON Schema describing the accepted parameters.
    fn parameters_schema(&self) -> serde_json::Value;

    /// Execute the tool with the given parameters.
    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value>;
}
