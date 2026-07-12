// Copyright 2025 Rararulab
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
mod artifacts;
mod ask_user;
mod bash;
mod create_directory;
mod debug_trace;
mod delete_file;
mod discover;
mod edit_file;
mod fff_find;
mod fff_grep;
mod file_stats;
mod finance_diagnostics;
mod finance_feed;
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
mod mita_write_skill_draft;
mod mita_write_user_note;
mod multi_edit;
mod notify;
mod path_check;
mod read_file;
pub mod run_code;
mod send_email;
mod send_file;
mod session_info;
mod set_avatar;
mod settings;
mod skill_tools;
mod system_paths;
mod timeout;
mod user_note;
mod walk_directory;
mod wechat_login;
mod write_file;

use acp_delegate::AcpDelegateTool;
use acp_tools::{InstallAcpAgentTool, ListAcpAgentsTool, RemoveAcpAgentTool};
use artifacts::ArtifactsTool;
use ask_user::AskUserTool;
use bash::BashTool;
use create_directory::CreateDirectoryTool;
use debug_trace::DebugTraceTool;
use delete_file::DeleteFileTool;
pub use discover::DiscoverToolsTool;
use edit_file::EditFileTool;
use fff_find::FffFindTool;
use fff_grep::FffGrepTool;
use file_stats::FileStatsTool;
use finance_diagnostics::FinanceDiagnoseCandleSubscriptionsTool;
use finance_feed::{
    FinanceDisableFeedSourceTool, FinanceEnableFeedSourceTool, FinanceListFeedSourcesTool,
    FinanceListSubscriptionsTool, FinanceRestartFeedSourceTool, FinanceSubscribeInstrumentsTool,
    FinanceSubscribeNewsTool, FinanceUnsubscribeTool,
};
use find_files::FindFilesTool;
use grep::GrepTool;
use http_fetch::HttpFetchTool;
use list_directory::ListDirectoryTool;
use marketplace::{
    MarketplaceAddSourceTool, MarketplaceBrowseTool, MarketplaceInstallTool,
    MarketplaceRefreshTool, MarketplaceSearchTool, MarketplaceUninstallTool,
};
use mcp_tools::{InstallMcpServerTool, ListMcpServersTool, RemoveMcpServerTool};
pub use mita_dispatch_rara::DispatchRaraTool;
use mita_distill_user_notes::DistillUserNotesTool;
use mita_evolve_soul::EvolveSoulTool;
use mita_list_sessions::ListSessionsTool;
use mita_read_tape::ReadTapeTool;
use mita_update_session_title::UpdateSessionTitleTool;
use mita_update_soul_state::UpdateSoulStateTool;
use mita_write_skill_draft::WriteSkillDraftTool;
use mita_write_user_note::MitaWriteUserNoteTool;
use multi_edit::MultiEditTool;
use rara_trading::finance::tools::FinanceSubscribeTool;
use read_file::ReadFileTool;
use run_code::RunCodeTool;
pub use run_code::SandboxCleanupHook;
use send_email::SendEmailTool;
use send_file::SendFileTool;
use session_info::SessionInfoTool;
use set_avatar::SetAvatarTool;
use settings::SettingsTool;
use skill_tools::{CreateSkillTool, DeleteSkillTool, ListSkillsTool};
use system_paths::SystemPathsTool;
use user_note::UserNoteTool;
use walk_directory::WalkDirectoryTool;
use wechat_login::{WechatLoginConfirmTool, WechatLoginStartTool};
use write_file::WriteFileTool;

// Re-export at the legacy path (`crate::tools::SandboxMap`) so existing
// callers in `boot.rs` and downstream tests keep compiling.
pub use crate::sandbox::SandboxMap;

/// Tool names for the rara agent manifest — single source of truth.
///
/// Only **Core** tools appear here. All other tools are registered in the
/// [`ToolRegistry`] but marked `tier = "deferred"` and discovered on demand
/// via the `discover-tools` tool.
pub fn rara_tool_names() -> Vec<rara_kernel::tool::ToolName> {
    use rara_kernel::{tool::ToolName, tool_names};

    vec![
        // File operations
        ToolName::new(BashTool::TOOL_NAME),
        ToolName::new(GrepTool::TOOL_NAME),
        ToolName::new(ReadFileTool::TOOL_NAME),
        ToolName::new(WriteFileTool::TOOL_NAME),
        ToolName::new(EditFileTool::TOOL_NAME),
        ToolName::new(ListDirectoryTool::TOOL_NAME),
        ToolName::new(FindFilesTool::TOOL_NAME),
        // Tape memory (2 Core; info/anchors/entries/between/checkout are Deferred)
        tool_names::TAPE_ANCHOR.clone(),
        tool_names::TAPE_SEARCH.clone(),
        // Background task delegation
        tool_names::TASK.clone(),
        tool_names::SPAWN_BACKGROUND.clone(),
        // Discovery
        ToolName::new(DiscoverToolsTool::TOOL_NAME),
    ]
}

/// Dependencies required to construct all tools.
pub struct ToolDeps {
    pub settings:              Arc<dyn rara_domain_shared::settings::SettingsProvider>,
    pub skill_registry:        rara_skills::registry::InMemoryRegistry,
    pub mcp_manager:           rara_mcp::manager::mgr::McpManager,
    pub tape_service:          rara_kernel::memory::TapeService,
    pub session_index:         rara_kernel::session::SessionIndexRef,
    pub marketplace_service:   std::sync::Arc<rara_skills::marketplace::MarketplaceService>,
    pub clawhub_client:        std::sync::Arc<rara_skills::clawhub::ClawhubClient>,
    pub acp_registry:          rara_acp::AcpRegistryRef,
    pub user_question_manager: rara_kernel::user_question::UserQuestionManagerRef,
    /// Shared fff file picker state (initialized at boot).
    pub fff_picker:            fff_search::SharedPicker,
    /// Shared fff query tracker state (initialized at boot).
    pub fff_query_tracker:     fff_search::SharedQueryTracker,
    /// Sandbox tool config from YAML; `None` disables `run_code`.
    pub sandbox_config:        Option<crate::SandboxToolConfig>,
    /// Shared per-session sandbox map; the cleanup hook holds a clone.
    pub sandbox_map:           SandboxMap,
    /// Finance information subscription registry.
    pub finance_registry:      Arc<rara_trading::finance::registry::FinanceSubscriptionRegistry>,
    /// Persistent data-feed service for finance feed source enabling.
    pub data_feed_svc:         rara_backend_admin::data_feeds::DataFeedSvc,
    /// Runtime data-feed registry for finance feed source enabling.
    pub data_feed_registry:    Arc<rara_kernel::data_feed::DataFeedRegistry>,
    /// Shared market-data repository for closed OHLCV candles.
    pub market_data_repo:      rara_trading::market_data::MarketDataRepositoryRef,
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
    let finance_tools = finance_tools(&deps);

    // Core tools.
    //
    // FS boundary: `bash` and `run_code` execute inside per-session boxlite
    // microVMs (see `crates/rara-sandbox/AGENT.md`). Read-side file tools
    // run on the host; write-side tools use canonical-path validation
    // against `rara_paths::workspace_dir()` to defeat both absolute-path
    // escape and symlink escape (#1936).
    let tools: Vec<AgentToolRef> = vec![
        Arc::new(BashTool::new(
            deps.sandbox_config.clone(),
            deps.sandbox_map.clone(),
        )),
        Arc::new(RunCodeTool::new(
            deps.sandbox_config.clone(),
            deps.sandbox_map.clone(),
        )),
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
        Arc::new(SendFileTool::new()),
        Arc::new(SetAvatarTool::new(deps.settings.clone())),
        Arc::new(SettingsTool::new(deps.settings.clone())),
        // Skill tools
        Arc::new(ListSkillsTool::new(deps.skill_registry.clone())),
        Arc::new(CreateSkillTool::new(deps.skill_registry.clone())),
        Arc::new(DeleteSkillTool::new(deps.skill_registry)),
        // Marketplace
        Arc::new(MarketplaceBrowseTool::new(
            deps.marketplace_service.clone(),
            deps.clawhub_client.clone(),
        )),
        Arc::new(MarketplaceSearchTool::new(
            deps.marketplace_service.clone(),
            deps.clawhub_client.clone(),
        )),
        Arc::new(MarketplaceInstallTool::new(
            deps.marketplace_service.clone(),
            deps.clawhub_client,
        )),
        Arc::new(MarketplaceUninstallTool::new(
            deps.marketplace_service.clone(),
        )),
        Arc::new(MarketplaceAddSourceTool::new(
            deps.marketplace_service.clone(),
        )),
        Arc::new(MarketplaceRefreshTool::new(deps.marketplace_service)),
        // MCP management tools
        Arc::new(InstallMcpServerTool::new(deps.mcp_manager.clone())),
        Arc::new(ListMcpServersTool::new(deps.mcp_manager.clone())),
        Arc::new(RemoveMcpServerTool::new(deps.mcp_manager)),
        // Tape management tools (tape-info/anchor/search/etc. are kernel-registered)
        Arc::new(DebugTraceTool::new(deps.tape_service.clone())),
        // User memory
        Arc::new(UserNoteTool::new(deps.tape_service.clone())),
        // Session info
        Arc::new(SessionInfoTool::new(deps.session_index.clone())),
        // System paths (directory layout discovery)
        Arc::new(SystemPathsTool::new()),
        // fff frecency-aware search tools (deferred tier)
        Arc::new(FffFindTool::new(
            deps.fff_picker.clone(),
            deps.fff_query_tracker.clone(),
        )),
        Arc::new(FffGrepTool::new(deps.fff_picker.clone())),
        // Mita-exclusive tools
        list_sessions,
        Arc::new(ReadTapeTool::new(deps.tape_service.clone())),
        Arc::new(MitaWriteUserNoteTool::new(deps.tape_service.clone())),
        Arc::new(DistillUserNotesTool::new(deps.tape_service.clone())),
        // Mita skill-draft tool
        Arc::new(WriteSkillDraftTool::new()),
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
        // User interaction
        Arc::new(AskUserTool::new(deps.user_question_manager)),
        // Artifacts (rich-content side panel — deferred tier)
        Arc::new(ArtifactsTool::new(deps.tape_service.clone())),
    ];

    for tool in tools.into_iter().chain(finance_tools) {
        registry.register(tool);
    }

    ToolRegistrationResult {
        dispatch_rara_handle: dispatch_handle_ref,
        list_sessions_handle: list_sessions_handle_ref,
    }
}

fn finance_tools(deps: &ToolDeps) -> Vec<AgentToolRef> {
    let mut tools: Vec<AgentToolRef> = vec![
        Arc::new(FinanceListFeedSourcesTool::new(
            deps.data_feed_svc.clone(),
            deps.data_feed_registry.clone(),
            deps.finance_registry.clone(),
        )),
        Arc::new(FinanceListSubscriptionsTool::new(
            deps.data_feed_svc.clone(),
            deps.data_feed_registry.clone(),
            deps.finance_registry.clone(),
        )),
        Arc::new(FinanceEnableFeedSourceTool::new(
            deps.data_feed_svc.clone(),
            deps.data_feed_registry.clone(),
        )),
        Arc::new(FinanceDisableFeedSourceTool::new(
            deps.data_feed_svc.clone(),
            deps.data_feed_registry.clone(),
        )),
        Arc::new(FinanceRestartFeedSourceTool::new(
            deps.data_feed_svc.clone(),
            deps.data_feed_registry.clone(),
        )),
        Arc::new(FinanceSubscribeInstrumentsTool::new(
            deps.data_feed_svc.clone(),
            deps.data_feed_registry.clone(),
            deps.finance_registry.clone(),
        )),
        Arc::new(FinanceSubscribeNewsTool::new(
            deps.data_feed_svc.clone(),
            deps.data_feed_registry.clone(),
            deps.finance_registry.clone(),
        )),
        Arc::new(FinanceDiagnoseCandleSubscriptionsTool::new(
            deps.data_feed_svc.clone(),
            deps.data_feed_registry.clone(),
            deps.finance_registry.clone(),
            deps.market_data_repo.clone(),
        )),
        Arc::new(FinanceSubscribeTool::new(deps.finance_registry.clone())),
        Arc::new(FinanceUnsubscribeTool::new(deps.finance_registry.clone())),
    ];
    tools.extend(finance_market_data_tools(&deps.market_data_repo));
    tools
}

fn finance_market_data_tools(
    market_data_repo: &rara_trading::market_data::MarketDataRepositoryRef,
) -> Vec<AgentToolRef> {
    vec![
        Arc::new(
            rara_trading::market_data::tools::FinanceListCandleStreamsTool::new(
                market_data_repo.clone(),
            ),
        ),
        Arc::new(
            rara_trading::market_data::tools::FinanceGetLatestCandleTool::new(
                market_data_repo.clone(),
            ),
        ),
        Arc::new(
            rara_trading::market_data::tools::FinanceGetRecentCandlesTool::new(
                market_data_repo.clone(),
            ),
        ),
        Arc::new(
            rara_trading::market_data::tools::FinanceQueryCandlesTool::new(
                market_data_repo.clone(),
            ),
        ),
        Arc::new(
            rara_trading::market_data::tools::FinanceFindCandleGapsTool::new(
                market_data_repo.clone(),
            ),
        ),
        Arc::new(
            rara_trading::market_data::tools::FinanceGetCandleFreshnessTool::new(
                market_data_repo.clone(),
            ),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use rara_kernel::{
        io::MessageId,
        queue::{ShardedEventQueue, ShardedEventQueueConfig},
        session::SessionKey,
        tool::{AgentTool, DiscoverToolsResult, DiscoverToolsStatus, ToolContext},
    };

    use super::*;

    fn context_with_registry(tool_registry: ToolRegistry) -> ToolContext {
        ToolContext {
            user_id:               "alice".to_owned(),
            session_key:           SessionKey::new(),
            origin_endpoint:       None,
            origin_user_id:        None,
            event_queue:           Arc::new(ShardedEventQueue::new(ShardedEventQueueConfig {
                num_shards:      0,
                shard_capacity:  1,
                global_capacity: 16,
            })),
            rara_turn_id:          MessageId::new(),
            context_window_tokens: 0,
            tool_registry:         Some(Arc::new(tool_registry)),
            stream_handle:         None,
            tool_call_id:          None,
        }
    }

    #[test]
    fn rara_tool_names_includes_core_tools() {
        let names = rara_tool_names();
        // Only Core tools appear in the manifest; deferred tools (kernel,
        // marketplace, schedule-*, etc.) are discovered on demand.
        for expected in [
            "bash",
            "tape-anchor",
            "tape-search",
            "task",
            "spawn-background",
            "discover-tools",
        ] {
            assert!(names.iter().any(|n| n == expected), "missing: {expected}");
        }
        // Verify deferred tools are NOT in the core list.
        for deferred in [
            "kernel",
            "marketplace-browse",
            "marketplace-search",
            "marketplace-install",
            "marketplace-uninstall",
            "marketplace-add-source",
            "marketplace-refresh",
            "schedule-once",
            "send-email",
            "memory",
            "http-fetch",
            "ask-user",
            "fff-find",
            "fff-grep",
            "finance_list_feed_sources",
            "finance_list_subscriptions",
            "finance_enable_feed_source",
            "finance_disable_feed_source",
            "finance_restart_feed_source",
            "finance_subscribe_instruments",
            "finance_subscribe_news",
            "finance_diagnose_candle_subscriptions",
            "finance_list_candle_streams",
            "finance_get_latest_candle",
            "finance_get_recent_candles",
            "finance_query_candles",
            "finance_find_candle_gaps",
            "finance_get_candle_freshness",
        ] {
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
            names.len() <= 12,
            "Core tool set has {} tools — keep it under 12 to control token costs. Use tier = \
             \"deferred\" for non-essential tools.",
            names.len()
        );
    }

    #[test]
    fn finance_market_data_tools_are_registered_as_deferred_tools() {
        let market_data_repo: rara_trading::market_data::MarketDataRepositoryRef =
            Arc::new(rara_trading::market_data::InMemoryMarketDataRepository::default());
        let mut registry = ToolRegistry::new();
        for tool in finance_market_data_tools(&market_data_repo) {
            registry.register(tool);
        }

        let names = registry.deferred_names();
        for expected in [
            "finance_list_candle_streams",
            "finance_get_latest_candle",
            "finance_get_recent_candles",
            "finance_query_candles",
            "finance_find_candle_gaps",
            "finance_get_candle_freshness",
        ] {
            assert!(
                names.iter().any(|name| name == expected),
                "missing finance market-data deferred tool: {expected}"
            );
        }
        assert_eq!(
            names.len(),
            6,
            "finance market-data helper should only register read-side candle tools"
        );
    }

    #[tokio::test]
    async fn discover_tools_finds_finance_market_data_candle_tools() {
        let market_data_repo: rara_trading::market_data::MarketDataRepositoryRef =
            Arc::new(rara_trading::market_data::InMemoryMarketDataRepository::default());
        let mut registry = ToolRegistry::new();
        for tool in finance_market_data_tools(&market_data_repo) {
            registry.register(tool);
        }
        let context = context_with_registry(registry);
        let discover = DiscoverToolsTool::new(rara_skills::registry::InMemoryRegistry::new());

        let output = discover
            .execute(serde_json::json!({"query": "candle"}), &context)
            .await
            .expect("discover finance market-data tools");
        let result: DiscoverToolsResult =
            serde_json::from_value(output.json).expect("discover result");

        assert_eq!(result.status, DiscoverToolsStatus::Activated);
        let names: Vec<&str> = result.tools.iter().map(|tool| tool.name.as_str()).collect();
        for expected in [
            "finance_list_candle_streams",
            "finance_get_latest_candle",
            "finance_get_recent_candles",
            "finance_query_candles",
            "finance_find_candle_gaps",
            "finance_get_candle_freshness",
        ] {
            assert!(
                names.contains(&expected),
                "discover-tools did not return finance market-data tool: {expected}"
            );
        }
    }
}
