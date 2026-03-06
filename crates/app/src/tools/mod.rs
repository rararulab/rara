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

//! Tool implementations and registration.

use std::{path::PathBuf, sync::Arc};

use rara_kernel::tool::{AgentToolRef, ToolRegistry};

mod bash;
mod composio;
mod edit_file;
mod find_files;
mod grep;
mod http_fetch;
mod list_directory;
mod mcp_tools;
mod mita_dispatch_rara;
mod mita_list_sessions;
mod mita_read_tape;
mod mita_write_user_note;
mod read_file;
mod screenshot;
mod send_email;
mod skill_tools;
mod user_note;
mod write_file;

use bash::BashTool;
use composio::ComposioTool;
use edit_file::EditFileTool;
use find_files::FindFilesTool;
use grep::GrepTool;
use http_fetch::HttpFetchTool;
use list_directory::ListDirectoryTool;
use mcp_tools::{InstallMcpServerTool, ListMcpServersTool, RemoveMcpServerTool};
use read_file::ReadFileTool;
use screenshot::ScreenshotTool;
use send_email::SendEmailTool;
use skill_tools::{CreateSkillTool, DeleteSkillTool, ListSkillsTool};
use user_note::UserNoteTool;
use write_file::WriteFileTool;

pub use mita_dispatch_rara::DispatchRaraTool;
use mita_list_sessions::ListSessionsTool;
use mita_read_tape::ReadTapeTool;
use mita_write_user_note::MitaWriteUserNoteTool;

/// Dependencies required to construct all tools.
pub struct ToolDeps {
    pub settings:               Arc<dyn rara_domain_shared::settings::SettingsProvider>,
    pub composio_auth_provider: Arc<dyn rara_composio::ComposioAuthProvider>,
    pub skill_registry:         rara_skills::registry::InMemoryRegistry,
    pub mcp_manager:            rara_mcp::manager::mgr::McpManager,
    pub tape_service:           rara_kernel::memory::TapeService,
    pub session_index:          Arc<dyn rara_kernel::session::SessionIndex>,
}

/// Result of tool registration, carrying handles needed for post-init wiring.
pub struct ToolRegistrationResult {
    /// Handle reference for the `DispatchRaraTool`, to be wired with the
    /// `KernelHandle` after kernel startup.
    pub dispatch_rara_handle: std::sync::Arc<tokio::sync::RwLock<Option<rara_kernel::handle::KernelHandle>>>,
}

/// Register all tools into the given [`ToolRegistry`].
///
/// Returns a [`ToolRegistrationResult`] containing handles that must be
/// wired after kernel startup (e.g. the `DispatchRaraTool` needs a
/// `KernelHandle`).
pub fn register_all(registry: &mut ToolRegistry, deps: ToolDeps) -> ToolRegistrationResult {
    let project_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    // Mita tools — constructed first so we can capture the dispatch handle.
    let dispatch_rara = Arc::new(DispatchRaraTool::new(deps.tape_service.clone()));
    let dispatch_handle_ref = dispatch_rara.handle_ref();

    // Core tools
    let tools: Vec<AgentToolRef> = vec![
        Arc::new(BashTool::new()),
        Arc::new(ReadFileTool::new()),
        Arc::new(WriteFileTool::new()),
        Arc::new(EditFileTool::new()),
        Arc::new(FindFilesTool::new()),
        Arc::new(GrepTool::new()),
        Arc::new(ListDirectoryTool::new()),
        Arc::new(HttpFetchTool::new()),
        Arc::new(SendEmailTool::new(deps.settings.clone())),
        Arc::new(ComposioTool::from_auth_provider(deps.composio_auth_provider)),
        // Screenshot
        Arc::new(ScreenshotTool::new(project_root)),
        // Skill tools
        Arc::new(ListSkillsTool::new(deps.skill_registry.clone())),
        Arc::new(CreateSkillTool::new(deps.skill_registry.clone())),
        Arc::new(DeleteSkillTool::new(deps.skill_registry)),
        // MCP management tools
        Arc::new(InstallMcpServerTool::new(deps.mcp_manager.clone())),
        Arc::new(ListMcpServersTool::new(deps.mcp_manager.clone())),
        Arc::new(RemoveMcpServerTool::new(deps.mcp_manager)),
        // User memory
        Arc::new(UserNoteTool::new(deps.tape_service.clone())),
        // Mita-exclusive tools
        Arc::new(ListSessionsTool::new(deps.session_index)),
        Arc::new(ReadTapeTool::new(deps.tape_service.clone())),
        Arc::new(MitaWriteUserNoteTool::new(deps.tape_service)),
        dispatch_rara,
    ];

    for tool in tools {
        registry.register(tool);
    }

    ToolRegistrationResult {
        dispatch_rara_handle: dispatch_handle_ref,
    }
}
