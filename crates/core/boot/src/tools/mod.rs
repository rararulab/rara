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

//! Tool implementations and factory functions.
//!
//! - **Primitives** (Layer 1): basic I/O, file operations, HTTP, etc.
//! - **Services** (Layer 2): complex business workflows built on domain services.

use std::path::PathBuf;
use std::sync::Arc;

use rara_domain_shared::settings::SettingsProvider;
use rara_kernel::tool::{AgentToolRef, ToolRegistry};

pub mod services;

mod bash;
mod composio;
mod edit_file;
mod find_files;
mod grep;
mod http_fetch;
mod list_directory;
#[cfg(feature = "k8s")]
pub mod pod;
mod read_file;
mod send_email;
mod storage_read;
mod write_file;

pub use bash::BashTool;
pub use composio::ComposioTool;
pub use edit_file::EditFileTool;
pub use find_files::FindFilesTool;
pub use grep::GrepTool;
pub use http_fetch::HttpFetchTool;
pub use list_directory::ListDirectoryTool;
#[cfg(feature = "k8s")]
pub use pod::PodTool;
pub use read_file::ReadFileTool;
pub use send_email::SendEmailTool;
pub use storage_read::StorageReadTool;
pub use write_file::WriteFileTool;

/// Dependencies required to construct primitive tools.
pub struct PrimitiveDeps {
    pub settings:               Arc<dyn SettingsProvider>,
    pub object_store:           opendal::Operator,
    pub composio_auth_provider: Arc<dyn rara_composio::ComposioAuthProvider>,
}

/// Returns all primitive tools, ready for registration.
pub fn default_primitives(deps: PrimitiveDeps) -> Vec<AgentToolRef> {
    let mut tools: Vec<AgentToolRef> = vec![
        // Core primitives
        Arc::new(BashTool::new()),
        Arc::new(ReadFileTool::new()),
        Arc::new(WriteFileTool::new()),
        Arc::new(EditFileTool::new()),
        Arc::new(FindFilesTool::new()),
        Arc::new(GrepTool::new()),
        Arc::new(ListDirectoryTool::new()),
        Arc::new(HttpFetchTool::new()),
        // Domain primitives
        Arc::new(SendEmailTool::new(deps.settings.clone())),
        Arc::new(StorageReadTool::new(deps.object_store)),
    ];
    tools.push(Arc::new(ComposioTool::from_auth_provider(
        deps.composio_auth_provider,
    )));
    tools
}

// ---------------------------------------------------------------------------
// Layer 2: Service tools
// ---------------------------------------------------------------------------

/// Dependencies required to construct Layer 2 service tools.
pub struct ServiceToolDeps {
    pub memory_manager:     Arc<rara_memory::MemoryManager>,
    pub recall_engine:      Arc<rara_memory::RecallStrategyEngine>,
    pub coding_task_service: rara_coding_task::service::CodingTaskService,
    pub skill_registry:     rara_skills::registry::InMemoryRegistry,
    pub mcp_manager:        rara_mcp::manager::mgr::McpManager,
    pub notify_client:      rara_domain_shared::notify::client::NotifyClient,
    pub settings:           Arc<dyn SettingsProvider>,
}

/// Register all Layer 2 service tools into the given [`ToolRegistry`].
pub fn register_service_tools(registry: &mut ToolRegistry, deps: ServiceToolDeps) {
    let project_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    // Memory tools
    registry.register_service(Arc::new(services::MemorySearchTool::new(Arc::clone(&deps.memory_manager))));
    registry.register_service(Arc::new(services::MemoryDeepRecallTool::new(Arc::clone(&deps.memory_manager))));
    registry.register_service(Arc::new(services::MemoryWriteTool::new(Arc::clone(&deps.memory_manager))));
    registry.register_service(Arc::new(services::MemoryAddFactTool::new(Arc::clone(&deps.memory_manager))));

    // Codex tools
    registry.register_service(Arc::new(services::CodexRunTool::new(deps.coding_task_service.clone())));
    registry.register_service(Arc::new(services::CodexStatusTool::new(deps.coding_task_service.clone())));
    registry.register_service(Arc::new(services::CodexListTool::new(deps.coding_task_service)));

    // Screenshot
    registry.register_service(Arc::new(services::ScreenshotTool::new(
        deps.notify_client,
        deps.settings,
        project_root,
    )));

    // Skill tools
    registry.register_service(Arc::new(services::ListSkillsTool::new(deps.skill_registry.clone())));
    registry.register_service(Arc::new(services::CreateSkillTool::new(deps.skill_registry.clone())));
    registry.register_service(Arc::new(services::DeleteSkillTool::new(deps.skill_registry)));

    // MCP tools
    registry.register_service(Arc::new(services::InstallMcpServerTool::new(deps.mcp_manager.clone())));
    registry.register_service(Arc::new(services::ListMcpServersTool::new(deps.mcp_manager.clone())));
    registry.register_service(Arc::new(services::RemoveMcpServerTool::new(deps.mcp_manager)));

    // Recall strategy tools
    registry.register_service(Arc::new(services::RecallStrategyAddTool::new(Arc::clone(&deps.recall_engine))));
    registry.register_service(Arc::new(services::RecallStrategyListTool::new(Arc::clone(&deps.recall_engine))));
    registry.register_service(Arc::new(services::RecallStrategyUpdateTool::new(Arc::clone(&deps.recall_engine))));
    registry.register_service(Arc::new(services::RecallStrategyRemoveTool::new(deps.recall_engine)));
}
