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

//! Stable identifier registry for tool names, guard rule names, and the
//! agent name namespace.
//!
//! Internal call sites that emit telemetry attribute *values* (e.g.
//! `tool.name`, `rara.guard.rule`) MUST use a constant from this module
//! rather than hardcoding the string. That way, renaming a tool in its
//! `#[tool(name = "...")]` definition is a compile-time error at every
//! telemetry call site instead of a silent contract break for the
//! external detector agent.
//!
//! Skills are intentionally NOT enumerated here — they are user-installed,
//! filesystem-loaded artifacts whose set is dynamic. The `rara.skill.name`
//! attribute carries whatever the loaded skill's frontmatter declares.

// ---------------------------------------------------------------------------
// Agent names
// ---------------------------------------------------------------------------

/// The user-facing rara agent.
pub const AGENT_RARA: &str = "rara";

/// The background mita agent (memory + soul curation).
pub const AGENT_MITA: &str = "mita";

// ---------------------------------------------------------------------------
// Guard rule names — keep in sync with `crates/kernel/src/guard/pattern.rs`.
// ---------------------------------------------------------------------------

/// Prompt-injection / system-prompt override pattern.
pub const GUARD_RULE_PROMPT_OVERRIDE: &str = "prompt_override";

/// Destructive shell pattern (rm -rf, format, etc.).
pub const GUARD_RULE_SHELL_DESTRUCTIVE: &str = "shell_destructive";

/// Data exfiltration pattern (curl + secrets, base64-encoded payloads).
pub const GUARD_RULE_DATA_EXFILTRATION: &str = "data_exfiltration";

/// Privilege escalation pattern (sudo, setuid, capability grant).
pub const GUARD_RULE_PRIVILEGE_ESCALATION: &str = "privilege_escalation";

// ---------------------------------------------------------------------------
// Tool names — generated from `#[tool(name = "...")]` declarations across the
// workspace. Test-only tools (`add`, `echo`) are excluded.
// ---------------------------------------------------------------------------

/// Tool `acp-delegate`.
pub const TOOL_ACP_DELEGATE: &str = "acp-delegate";

/// Tool `artifacts`.
pub const TOOL_ARTIFACTS: &str = "artifacts";

/// Tool `ask-user`.
pub const TOOL_ASK_USER: &str = "ask-user";

/// Tool `bash`.
pub const TOOL_BASH: &str = "bash";

/// Tool `browser-click`.
pub const TOOL_BROWSER_CLICK: &str = "browser-click";

/// Tool `browser-close`.
pub const TOOL_BROWSER_CLOSE: &str = "browser-close";

/// Tool `browser-evaluate`.
pub const TOOL_BROWSER_EVALUATE: &str = "browser-evaluate";

/// Tool `browser-fetch`.
pub const TOOL_BROWSER_FETCH: &str = "browser-fetch";

/// Tool `browser-navigate-back`.
pub const TOOL_BROWSER_NAVIGATE_BACK: &str = "browser-navigate-back";

/// Tool `browser-navigate`.
pub const TOOL_BROWSER_NAVIGATE: &str = "browser-navigate";

/// Tool `browser-press-key`.
pub const TOOL_BROWSER_PRESS_KEY: &str = "browser-press-key";

/// Tool `browser-snapshot`.
pub const TOOL_BROWSER_SNAPSHOT: &str = "browser-snapshot";

/// Tool `browser-tabs`.
pub const TOOL_BROWSER_TABS: &str = "browser-tabs";

/// Tool `browser-type`.
pub const TOOL_BROWSER_TYPE: &str = "browser-type";

/// Tool `browser-wait-for`.
pub const TOOL_BROWSER_WAIT_FOR: &str = "browser-wait-for";

/// Tool `cancel-background`.
pub const TOOL_CANCEL_BACKGROUND: &str = "cancel-background";

/// Tool `composio_accounts`.
pub const TOOL_COMPOSIO_ACCOUNTS: &str = "composio_accounts";

/// Tool `composio_connect`.
pub const TOOL_COMPOSIO_CONNECT: &str = "composio_connect";

/// Tool `composio_execute`.
pub const TOOL_COMPOSIO_EXECUTE: &str = "composio_execute";

/// Tool `composio_list`.
pub const TOOL_COMPOSIO_LIST: &str = "composio_list";

/// Tool `continue-work`.
pub const TOOL_CONTINUE_WORK: &str = "continue-work";

/// Tool `create-directory`.
pub const TOOL_CREATE_DIRECTORY: &str = "create-directory";

/// Tool `create-plan`.
pub const TOOL_CREATE_PLAN: &str = "create-plan";

/// Tool `create-skill`.
pub const TOOL_CREATE_SKILL: &str = "create-skill";

/// Tool `debug_trace`.
pub const TOOL_DEBUG_TRACE: &str = "debug_trace";

/// Tool `delete-file`.
pub const TOOL_DELETE_FILE: &str = "delete-file";

/// Tool `delete-skill`.
pub const TOOL_DELETE_SKILL: &str = "delete-skill";

/// Tool `discover-tools`.
pub const TOOL_DISCOVER_TOOLS: &str = "discover-tools";

/// Tool `dispatch-rara`.
pub const TOOL_DISPATCH_RARA: &str = "dispatch-rara";

/// Tool `distill-user-notes`.
pub const TOOL_DISTILL_USER_NOTES: &str = "distill-user-notes";

/// Tool `dock.annotation.add`.
pub const TOOL_DOCK_ANNOTATION_ADD: &str = "dock.annotation.add";

/// Tool `dock.annotation.remove`.
pub const TOOL_DOCK_ANNOTATION_REMOVE: &str = "dock.annotation.remove";

/// Tool `dock.annotation.update`.
pub const TOOL_DOCK_ANNOTATION_UPDATE: &str = "dock.annotation.update";

/// Tool `dock.block.add`.
pub const TOOL_DOCK_BLOCK_ADD: &str = "dock.block.add";

/// Tool `dock.block.remove`.
pub const TOOL_DOCK_BLOCK_REMOVE: &str = "dock.block.remove";

/// Tool `dock.block.update`.
pub const TOOL_DOCK_BLOCK_UPDATE: &str = "dock.block.update";

/// Tool `dock.fact.add`.
pub const TOOL_DOCK_FACT_ADD: &str = "dock.fact.add";

/// Tool `dock.fact.remove`.
pub const TOOL_DOCK_FACT_REMOVE: &str = "dock.fact.remove";

/// Tool `dock.fact.update`.
pub const TOOL_DOCK_FACT_UPDATE: &str = "dock.fact.update";

/// Tool `edit-file`.
pub const TOOL_EDIT_FILE: &str = "edit-file";

/// Tool `evolve-soul`.
pub const TOOL_EVOLVE_SOUL: &str = "evolve-soul";

/// Tool `fff-find`.
pub const TOOL_FFF_FIND: &str = "fff-find";

/// Tool `fff-grep`.
pub const TOOL_FFF_GREP: &str = "fff-grep";

/// Tool `file-stats`.
pub const TOOL_FILE_STATS: &str = "file-stats";

/// Tool `find-files`.
pub const TOOL_FIND_FILES: &str = "find-files";

/// Tool `fold-branch`.
pub const TOOL_FOLD_BRANCH: &str = "fold-branch";

/// Tool `get-session-info`.
pub const TOOL_GET_SESSION_INFO: &str = "get-session-info";

/// Tool `grep`.
pub const TOOL_GREP: &str = "grep";

/// Tool `http-fetch`.
pub const TOOL_HTTP_FETCH: &str = "http-fetch";

/// Tool `install-acp-agent`.
pub const TOOL_INSTALL_ACP_AGENT: &str = "install-acp-agent";

/// Tool `install-mcp-server`.
pub const TOOL_INSTALL_MCP_SERVER: &str = "install-mcp-server";

/// Tool `list-acp-agents`.
pub const TOOL_LIST_ACP_AGENTS: &str = "list-acp-agents";

/// Tool `list-directory`.
pub const TOOL_LIST_DIRECTORY: &str = "list-directory";

/// Tool `list-mcp-servers`.
pub const TOOL_LIST_MCP_SERVERS: &str = "list-mcp-servers";

/// Tool `list-sessions`.
pub const TOOL_LIST_SESSIONS: &str = "list-sessions";

/// Tool `list-skills`.
pub const TOOL_LIST_SKILLS: &str = "list-skills";

/// Tool `marketplace-add-source`.
pub const TOOL_MARKETPLACE_ADD_SOURCE: &str = "marketplace-add-source";

/// Tool `marketplace-browse`.
pub const TOOL_MARKETPLACE_BROWSE: &str = "marketplace-browse";

/// Tool `marketplace-install`.
pub const TOOL_MARKETPLACE_INSTALL: &str = "marketplace-install";

/// Tool `marketplace-refresh`.
pub const TOOL_MARKETPLACE_REFRESH: &str = "marketplace-refresh";

/// Tool `marketplace-search`.
pub const TOOL_MARKETPLACE_SEARCH: &str = "marketplace-search";

/// Tool `marketplace-uninstall`.
pub const TOOL_MARKETPLACE_UNINSTALL: &str = "marketplace-uninstall";

/// Tool `memory`.
pub const TOOL_MEMORY: &str = "memory";

/// Tool `multi-edit`.
pub const TOOL_MULTI_EDIT: &str = "multi-edit";

/// Tool `query-feed`.
pub const TOOL_QUERY_FEED: &str = "query-feed";

/// Tool `read-file`.
pub const TOOL_READ_FILE: &str = "read-file";

/// Tool `read-tape`.
pub const TOOL_READ_TAPE: &str = "read-tape";

/// Tool `remove-acp-agent`.
pub const TOOL_REMOVE_ACP_AGENT: &str = "remove-acp-agent";

/// Tool `remove-mcp-server`.
pub const TOOL_REMOVE_MCP_SERVER: &str = "remove-mcp-server";

/// Tool `schedule-cron`.
pub const TOOL_SCHEDULE_CRON: &str = "schedule-cron";

/// Tool `schedule-interval`.
pub const TOOL_SCHEDULE_INTERVAL: &str = "schedule-interval";

/// Tool `schedule-list`.
pub const TOOL_SCHEDULE_LIST: &str = "schedule-list";

/// Tool `schedule-once`.
pub const TOOL_SCHEDULE_ONCE: &str = "schedule-once";

/// Tool `schedule-remove`.
pub const TOOL_SCHEDULE_REMOVE: &str = "schedule-remove";

/// Tool `send-email`.
pub const TOOL_SEND_EMAIL: &str = "send-email";

/// Tool `send-file`.
pub const TOOL_SEND_FILE: &str = "send-file";

/// Tool `set-avatar`.
pub const TOOL_SET_AVATAR: &str = "set-avatar";

/// Tool `settings`.
pub const TOOL_SETTINGS: &str = "settings";

/// Tool `spawn-background`.
pub const TOOL_SPAWN_BACKGROUND: &str = "spawn-background";

/// Tool `system-paths`.
pub const TOOL_SYSTEM_PATHS: &str = "system-paths";

/// Tool `tape-anchor`.
pub const TOOL_TAPE_ANCHOR: &str = "tape-anchor";

/// Tool `tape-anchors`.
pub const TOOL_TAPE_ANCHORS: &str = "tape-anchors";

/// Tool `tape-between`.
pub const TOOL_TAPE_BETWEEN: &str = "tape-between";

/// Tool `tape-checkout-root`.
pub const TOOL_TAPE_CHECKOUT_ROOT: &str = "tape-checkout-root";

/// Tool `tape-checkout`.
pub const TOOL_TAPE_CHECKOUT: &str = "tape-checkout";

/// Tool `tape-entries`.
pub const TOOL_TAPE_ENTRIES: &str = "tape-entries";

/// Tool `tape-info`.
pub const TOOL_TAPE_INFO: &str = "tape-info";

/// Tool `tape-search`.
pub const TOOL_TAPE_SEARCH: &str = "tape-search";

/// Tool `task`.
pub const TOOL_TASK: &str = "task";

/// Tool `update-session-title`.
pub const TOOL_UPDATE_SESSION_TITLE: &str = "update-session-title";

/// Tool `update-soul-state`.
pub const TOOL_UPDATE_SOUL_STATE: &str = "update-soul-state";

/// Tool `user-note`.
pub const TOOL_USER_NOTE: &str = "user-note";

/// Tool `walk-directory`.
pub const TOOL_WALK_DIRECTORY: &str = "walk-directory";

/// Tool `wechat-login-confirm`.
pub const TOOL_WECHAT_LOGIN_CONFIRM: &str = "wechat-login-confirm";

/// Tool `wechat-login-start`.
pub const TOOL_WECHAT_LOGIN_START: &str = "wechat-login-start";

/// Tool `write-file`.
pub const TOOL_WRITE_FILE: &str = "write-file";

/// Tool `write-skill-draft`.
pub const TOOL_WRITE_SKILL_DRAFT: &str = "write-skill-draft";

/// Tool `write-user-note`.
pub const TOOL_WRITE_USER_NOTE: &str = "write-user-note";

/// All tool name constants exported by this module. Useful for tests that
/// assert the registry has not silently lost a tool.
pub const ALL_TOOL_NAMES: &[&str] = &[
    TOOL_ACP_DELEGATE,
    TOOL_ARTIFACTS,
    TOOL_ASK_USER,
    TOOL_BASH,
    TOOL_BROWSER_CLICK,
    TOOL_BROWSER_CLOSE,
    TOOL_BROWSER_EVALUATE,
    TOOL_BROWSER_FETCH,
    TOOL_BROWSER_NAVIGATE_BACK,
    TOOL_BROWSER_NAVIGATE,
    TOOL_BROWSER_PRESS_KEY,
    TOOL_BROWSER_SNAPSHOT,
    TOOL_BROWSER_TABS,
    TOOL_BROWSER_TYPE,
    TOOL_BROWSER_WAIT_FOR,
    TOOL_CANCEL_BACKGROUND,
    TOOL_COMPOSIO_ACCOUNTS,
    TOOL_COMPOSIO_CONNECT,
    TOOL_COMPOSIO_EXECUTE,
    TOOL_COMPOSIO_LIST,
    TOOL_CONTINUE_WORK,
    TOOL_CREATE_DIRECTORY,
    TOOL_CREATE_PLAN,
    TOOL_CREATE_SKILL,
    TOOL_DEBUG_TRACE,
    TOOL_DELETE_FILE,
    TOOL_DELETE_SKILL,
    TOOL_DISCOVER_TOOLS,
    TOOL_DISPATCH_RARA,
    TOOL_DISTILL_USER_NOTES,
    TOOL_DOCK_ANNOTATION_ADD,
    TOOL_DOCK_ANNOTATION_REMOVE,
    TOOL_DOCK_ANNOTATION_UPDATE,
    TOOL_DOCK_BLOCK_ADD,
    TOOL_DOCK_BLOCK_REMOVE,
    TOOL_DOCK_BLOCK_UPDATE,
    TOOL_DOCK_FACT_ADD,
    TOOL_DOCK_FACT_REMOVE,
    TOOL_DOCK_FACT_UPDATE,
    TOOL_EDIT_FILE,
    TOOL_EVOLVE_SOUL,
    TOOL_FFF_FIND,
    TOOL_FFF_GREP,
    TOOL_FILE_STATS,
    TOOL_FIND_FILES,
    TOOL_FOLD_BRANCH,
    TOOL_GET_SESSION_INFO,
    TOOL_GREP,
    TOOL_HTTP_FETCH,
    TOOL_INSTALL_ACP_AGENT,
    TOOL_INSTALL_MCP_SERVER,
    TOOL_LIST_ACP_AGENTS,
    TOOL_LIST_DIRECTORY,
    TOOL_LIST_MCP_SERVERS,
    TOOL_LIST_SESSIONS,
    TOOL_LIST_SKILLS,
    TOOL_MARKETPLACE_ADD_SOURCE,
    TOOL_MARKETPLACE_BROWSE,
    TOOL_MARKETPLACE_INSTALL,
    TOOL_MARKETPLACE_REFRESH,
    TOOL_MARKETPLACE_SEARCH,
    TOOL_MARKETPLACE_UNINSTALL,
    TOOL_MEMORY,
    TOOL_MULTI_EDIT,
    TOOL_QUERY_FEED,
    TOOL_READ_FILE,
    TOOL_READ_TAPE,
    TOOL_REMOVE_ACP_AGENT,
    TOOL_REMOVE_MCP_SERVER,
    TOOL_SCHEDULE_CRON,
    TOOL_SCHEDULE_INTERVAL,
    TOOL_SCHEDULE_LIST,
    TOOL_SCHEDULE_ONCE,
    TOOL_SCHEDULE_REMOVE,
    TOOL_SEND_EMAIL,
    TOOL_SEND_FILE,
    TOOL_SET_AVATAR,
    TOOL_SETTINGS,
    TOOL_SPAWN_BACKGROUND,
    TOOL_SYSTEM_PATHS,
    TOOL_TAPE_ANCHOR,
    TOOL_TAPE_ANCHORS,
    TOOL_TAPE_BETWEEN,
    TOOL_TAPE_CHECKOUT_ROOT,
    TOOL_TAPE_CHECKOUT,
    TOOL_TAPE_ENTRIES,
    TOOL_TAPE_INFO,
    TOOL_TAPE_SEARCH,
    TOOL_TASK,
    TOOL_UPDATE_SESSION_TITLE,
    TOOL_UPDATE_SOUL_STATE,
    TOOL_USER_NOTE,
    TOOL_WALK_DIRECTORY,
    TOOL_WECHAT_LOGIN_CONFIRM,
    TOOL_WECHAT_LOGIN_START,
    TOOL_WRITE_FILE,
    TOOL_WRITE_SKILL_DRAFT,
    TOOL_WRITE_USER_NOTE,
];

/// All guard rule name constants exported by this module.
pub const ALL_GUARD_RULES: &[&str] = &[
    GUARD_RULE_PROMPT_OVERRIDE,
    GUARD_RULE_SHELL_DESTRUCTIVE,
    GUARD_RULE_DATA_EXFILTRATION,
    GUARD_RULE_PRIVILEGE_ESCALATION,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_tool_names_are_unique() {
        let mut sorted: Vec<&str> = ALL_TOOL_NAMES.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            ALL_TOOL_NAMES.len(),
            "duplicate tool name in registry"
        );
    }

    #[test]
    fn all_guard_rules_are_unique() {
        let mut sorted: Vec<&str> = ALL_GUARD_RULES.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ALL_GUARD_RULES.len(), "duplicate guard rule");
    }

    #[test]
    fn agent_names_are_lowercase() {
        for name in [AGENT_RARA, AGENT_MITA] {
            assert_eq!(name, name.to_lowercase(), "agent name must be lowercase");
        }
    }
}
