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

use std::sync::Arc;

use rara_kernel::tool::{AgentToolRef, ToolRegistry};

mod acp_delegate;
mod acp_tools;
mod bash;
mod composio;
mod create_directory;
mod debug_trace;
mod delete_file;
mod discover;
mod edit_file;
mod file_stats;
mod find_files;
mod grep;
mod http_fetch;
mod list_directory;
mod marketplace;
mod mcp_tools;
mod mita_dispatch_rara;
mod mita_distill_user_notes;
mod mita_evolve_soul;
mod mita_list_sessions;
mod mita_read_tape;
mod mita_update_session_title;
mod mita_update_soul_state;
mod mita_write_user_note;
mod multi_edit;
mod notify;
mod read_file;
mod send_email;
mod send_image;
mod session_info;
mod set_avatar;
mod settings;
mod skill_tools;
mod tape_handoff;
mod tape_info;
mod user_note;
mod walk_directory;
mod wechat_login;
mod write_file;

use acp_delegate::AcpDelegateTool;
use acp_tools::{InstallAcpAgentTool, ListAcpAgentsTool, RemoveAcpAgentTool};
use bash::BashTool;
use create_directory::CreateDirectoryTool;
use debug_trace::DebugTraceTool;
use delete_file::DeleteFileTool;
pub use discover::DiscoverToolsTool;
use edit_file::EditFileTool;
use file_stats::FileStatsTool;
use find_files::FindFilesTool;
use grep::GrepTool;
use http_fetch::HttpFetchTool;
use list_directory::ListDirectoryTool;
use marketplace::MarketplaceTool;
use mcp_tools::{InstallMcpServerTool, ListMcpServersTool, RemoveMcpServerTool};
pub use mita_dispatch_rara::DispatchRaraTool;
use mita_distill_user_notes::DistillUserNotesTool;
use mita_evolve_soul::EvolveSoulTool;
use mita_list_sessions::ListSessionsTool;
use mita_read_tape::ReadTapeTool;
use mita_update_session_title::UpdateSessionTitleTool;
use mita_update_soul_state::UpdateSoulStateTool;
use mita_write_user_note::MitaWriteUserNoteTool;
use multi_edit::MultiEditTool;
use read_file::ReadFileTool;
use send_email::SendEmailTool;
use send_image::SendImageTool;
use session_info::SessionInfoTool;
use set_avatar::SetAvatarTool;
use settings::SettingsTool;
use skill_tools::{CreateSkillTool, DeleteSkillTool, ListSkillsTool};
use tape_handoff::TapeHandoffTool;
use tape_info::TapeInfoTool;
use user_note::UserNoteTool;
use walk_directory::WalkDirectoryTool;
use wechat_login::{WechatLoginConfirmTool, WechatLoginStartTool};
use write_file::WriteFileTool;

/// Tool names for the rara agent manifest — single source of truth.
///
/// Only **Core** tools appear here. All other tools are registered in the
/// [`ToolRegistry`] but marked `tier = "deferred"` and discovered on demand
/// via the `discover-tools` tool.
pub fn rara_tool_names() -> Vec<String> {
    use rara_kernel::tool_names;

    vec![
        // File operations
        BashTool::TOOL_NAME,
        GrepTool::TOOL_NAME,
        ReadFileTool::TOOL_NAME,
        WriteFileTool::TOOL_NAME,
        EditFileTool::TOOL_NAME,
        MultiEditTool::TOOL_NAME,
        ListDirectoryTool::TOOL_NAME,
        FindFilesTool::TOOL_NAME,
        WalkDirectoryTool::TOOL_NAME,
        FileStatsTool::TOOL_NAME,
        DeleteFileTool::TOOL_NAME,
        CreateDirectoryTool::TOOL_NAME,
        // Network
        HttpFetchTool::TOOL_NAME,
        // Memory & session
        tool_names::TAPE,
        UserNoteTool::TOOL_NAME,
        tool_names::MEMORY,
        // Execution
        tool_names::SPAWN_BACKGROUND,
        tool_names::CANCEL_BACKGROUND,
        // Plan
        tool_names::CREATE_PLAN,
        // Discovery
        DiscoverToolsTool::TOOL_NAME,
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

/// Dependencies required to construct all tools.
pub struct ToolDeps {
    pub settings:               Arc<dyn rara_domain_shared::settings::SettingsProvider>,
    pub composio_auth_provider: Arc<dyn rara_composio::ComposioAuthProvider>,
    pub skill_registry:         rara_skills::registry::InMemoryRegistry,
    pub mcp_manager:            rara_mcp::manager::mgr::McpManager,
    pub tape_service:           rara_kernel::memory::TapeService,
    pub session_index:          rara_kernel::session::SessionIndexRef,
    pub marketplace_service:    std::sync::Arc<rara_skills::marketplace::MarketplaceService>,
    pub clawhub_client:         std::sync::Arc<rara_skills::clawhub::ClawhubClient>,
    pub dock_mutation_sink:     rara_dock::DockMutationSink,
    pub acp_registry:           rara_acp::AcpRegistryRef,
}

/// Result of tool registration, carrying handles needed for post-init wiring.
pub struct ToolRegistrationResult {
    /// Handle reference for the `DispatchRaraTool`, to be wired with the
    /// `KernelHandle` after kernel startup.
    pub dispatch_rara_handle:
        std::sync::Arc<tokio::sync::RwLock<Option<rara_kernel::handle::KernelHandle>>>,
    /// Handle reference for the `ListSessionsTool`, to be wired with the
    /// `KernelHandle` after kernel startup.
    pub list_sessions_handle:
        std::sync::Arc<tokio::sync::RwLock<Option<rara_kernel::handle::KernelHandle>>>,
}

/// Register all tools into the given [`ToolRegistry`].
///
/// Returns a [`ToolRegistrationResult`] containing handles that must be
/// wired after kernel startup (e.g. the `DispatchRaraTool` needs a
/// `KernelHandle`).
pub fn register_all(registry: &mut ToolRegistry, deps: ToolDeps) -> ToolRegistrationResult {
    // Mita tools — constructed first so we can capture the handle refs.
    let dispatch_rara = Arc::new(DispatchRaraTool::new(deps.tape_service.clone()));
    let dispatch_handle_ref = dispatch_rara.handle_ref();
    let list_sessions = Arc::new(ListSessionsTool::new());
    let list_sessions_handle_ref = list_sessions.handle_ref();

    // Core tools
    // SYNC: file-access tools are guarded by PathScopeGuard. When adding a new
    // tool that reads/writes files, also add it to the constant arrays in
    // `rara_kernel::guard::path_scope::{FILE_PATH_TOOLS, PATH_TOOLS}`.
    let tools: Vec<AgentToolRef> = vec![
        Arc::new(BashTool::new()),
        Arc::new(ReadFileTool::new()),
        Arc::new(WriteFileTool::new()),
        Arc::new(EditFileTool::new()),
        Arc::new(MultiEditTool::new()),
        Arc::new(FindFilesTool::new()),
        Arc::new(GrepTool::new()),
        Arc::new(ListDirectoryTool::new()),
        Arc::new(WalkDirectoryTool::new()),
        Arc::new(FileStatsTool::new()),
        Arc::new(DeleteFileTool::new()),
        Arc::new(CreateDirectoryTool::new()),
        Arc::new(HttpFetchTool::new()),
        Arc::new(SendEmailTool::new(deps.settings.clone())),
        Arc::new(SendImageTool::new()),
        Arc::new(SetAvatarTool::new(deps.settings.clone())),
        Arc::new(SettingsTool::new(deps.settings.clone())),
        // Skill tools
        Arc::new(ListSkillsTool::new(deps.skill_registry.clone())),
        Arc::new(CreateSkillTool::new(deps.skill_registry.clone())),
        Arc::new(DeleteSkillTool::new(deps.skill_registry)),
        // Marketplace
        Arc::new(MarketplaceTool::new(
            deps.marketplace_service,
            deps.clawhub_client,
        )),
        // MCP management tools
        Arc::new(InstallMcpServerTool::new(deps.mcp_manager.clone())),
        Arc::new(ListMcpServersTool::new(deps.mcp_manager.clone())),
        Arc::new(RemoveMcpServerTool::new(deps.mcp_manager)),
        // Tape management tools
        Arc::new(TapeInfoTool::new(deps.tape_service.clone())),
        Arc::new(TapeHandoffTool::new(deps.tape_service.clone())),
        Arc::new(DebugTraceTool::new(deps.tape_service.clone())),
        // User memory
        Arc::new(UserNoteTool::new(deps.tape_service.clone())),
        // Session info
        Arc::new(SessionInfoTool::new(deps.session_index.clone())),
        // Mita-exclusive tools
        list_sessions,
        Arc::new(ReadTapeTool::new(deps.tape_service.clone())),
        Arc::new(MitaWriteUserNoteTool::new(deps.tape_service.clone())),
        Arc::new(DistillUserNotesTool::new(deps.tape_service)),
        dispatch_rara,
        // Mita session management tools
        Arc::new(UpdateSessionTitleTool::new(deps.session_index.clone())),
        // Mita soul evolution tools
        Arc::new(UpdateSoulStateTool::new()),
        Arc::new(EvolveSoulTool::new()),
        // ACP delegation
        Arc::new(AcpDelegateTool::new(deps.acp_registry.clone())),
        // ACP management tools
        Arc::new(InstallAcpAgentTool::new(deps.acp_registry.clone())),
        Arc::new(ListAcpAgentsTool::new(deps.acp_registry.clone())),
        Arc::new(RemoveAcpAgentTool::new(deps.acp_registry)),
        // WeChat login (two-step: start → confirm)
        Arc::new(WechatLoginStartTool::new()),
        Arc::new(WechatLoginConfirmTool::new()),
    ];

    for tool in tools {
        registry.register(tool);
    }

    // Dock canvas tools (block, fact, annotation CRUD)
    for tool in rara_dock::dock_tools(deps.dock_mutation_sink) {
        registry.register(tool);
    }

    // Composio tool suite (4 focused tools)
    for tool in composio::build_tools(deps.composio_auth_provider) {
        registry.register(tool);
    }

    ToolRegistrationResult {
        dispatch_rara_handle: dispatch_handle_ref,
        list_sessions_handle: list_sessions_handle_ref,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rara_tool_names_includes_core_tools() {
        let names = rara_tool_names();
        // Only Core tools appear in the manifest; deferred tools (kernel,
        // marketplace, schedule-*, etc.) are discovered on demand.
        for expected in ["bash", "tape", "memory", "discover-tools"] {
            assert!(names.iter().any(|n| n == expected), "missing: {expected}");
        }
        // Verify deferred tools are NOT in the core list.
        for deferred in ["kernel", "marketplace", "schedule-once", "send-email"] {
            assert!(
                !names.iter().any(|n| n == deferred),
                "deferred tool should not be in core: {deferred}"
            );
        }
    }

    #[test]
    fn rara_core_tool_count_stays_slim() {
        let names = rara_tool_names();
        assert!(
            names.len() <= 22,
            "Core tool set has {} tools — keep it under 22 to control token costs. Use tier = \
             \"deferred\" for non-essential tools.",
            names.len()
        );
    }
}
