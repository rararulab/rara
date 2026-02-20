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
//!
//! It also houses all **primitive tool** implementations (core + domain) and
//! provides [`default_primitives`] to obtain them in one call.

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

/// Dependencies required to construct domain-level primitive tools.
pub struct PrimitiveDeps {
    pub pool:                   sqlx::PgPool,
    pub notify_client:          rara_domain_shared::notify::client::NotifyClient,
    pub settings_svc:           rara_domain_shared::settings::SettingsSvc,
    pub object_store:           opendal::Operator,
    pub composio_auth_provider: Arc<dyn rara_composio::ComposioAuthProvider>,
}

/// Returns all primitive tools (core + domain), ready for registration.
pub fn default_primitives(deps: PrimitiveDeps) -> Vec<AgentToolRef> {
    let mut tools = core_primitives_vec();
    tools.extend(domain_primitives_vec(deps));
    tools
}

/// Returns only the 8 core primitives (no application deps).
pub fn core_primitives_vec() -> Vec<AgentToolRef> {
    vec![
        Arc::new(core_primitives::BashTool::new()),
        Arc::new(core_primitives::ReadFileTool::new()),
        Arc::new(core_primitives::WriteFileTool::new()),
        Arc::new(core_primitives::EditFileTool::new()),
        Arc::new(core_primitives::FindFilesTool::new()),
        Arc::new(core_primitives::GrepTool::new()),
        Arc::new(core_primitives::ListDirectoryTool::new()),
        Arc::new(core_primitives::HttpFetchTool::new()),
    ]
}

/// Returns domain primitives. Composio is included when configured.
pub fn domain_primitives_vec(deps: PrimitiveDeps) -> Vec<AgentToolRef> {
    let mut tools: Vec<AgentToolRef> = vec![
        Arc::new(domain_primitives::DbQueryTool::new(deps.pool.clone())),
        Arc::new(domain_primitives::DbMutateTool::new(deps.pool)),
        Arc::new(domain_primitives::NotifyTool::new(
            deps.notify_client,
            deps.settings_svc,
        )),
        Arc::new(domain_primitives::StorageReadTool::new(deps.object_store)),
    ];
    tools.push(Arc::new(
        domain_primitives::ComposioTool::from_auth_provider(deps.composio_auth_provider),
    ));
    tools
}
