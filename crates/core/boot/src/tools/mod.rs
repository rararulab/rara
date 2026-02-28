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

//! Primitive tool implementations (core + domain) and factory functions.
//!
//! This module houses all **primitive tool** implementations and
//! provides [`default_primitives`] to obtain them in one call.

use std::sync::Arc;

use rara_domain_shared::settings::model::Settings;
use rara_kernel::tool::AgentToolRef;
use tokio::sync::watch;

pub mod core_primitives;
pub mod domain_primitives;

/// Dependencies required to construct domain-level primitive tools.
pub struct PrimitiveDeps {
    pub pool:                   sqlx::PgPool,
    pub notify_client:          rara_domain_shared::notify::client::NotifyClient,
    pub settings_rx:            watch::Receiver<Settings>,
    pub object_store:           opendal::Operator,
    pub composio_auth_provider: Arc<dyn rara_composio::ComposioAuthProvider>,
    pub contact_lookup:         Arc<dyn rara_kernel::contact_lookup::ContactLookup>,
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
            deps.settings_rx.clone(),
            deps.contact_lookup,
        )),
        Arc::new(domain_primitives::SendEmailTool::new(deps.settings_rx)),
        Arc::new(domain_primitives::StorageReadTool::new(deps.object_store)),
    ];
    tools.push(Arc::new(
        domain_primitives::ComposioTool::from_auth_provider(deps.composio_auth_provider),
    ));
    tools
}
