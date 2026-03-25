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

//! Shared helpers for rendering tool-call progress across adapters.

use std::str::FromStr;

use strum::EnumMessage;

/// Known tool kinds with display name and activity label.
///
/// - `serialize` = raw tool name from the LLM
/// - `detailed_message` = short English display name (used by web adapter,
///   logs)
/// - `message` = Chinese activity phrase (used by TG adapter for user-facing
///   progress)
///
/// Multiple raw names can map to the same variant via multiple `serialize`
/// attrs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumString, strum::EnumMessage)]
pub enum ToolKind {
    #[strum(serialize = "shell_execute")]
    #[strum(message = "执行命令", detailed_message = "shell")]
    ShellExecute,

    #[strum(serialize = "bash")]
    #[strum(message = "执行命令", detailed_message = "bash")]
    Bash,

    #[strum(serialize = "web_search")]
    #[strum(message = "搜索网页", detailed_message = "search")]
    WebSearch,

    #[strum(serialize = "web_fetch", serialize = "http-fetch")]
    #[strum(message = "获取网页", detailed_message = "fetch")]
    WebFetch,

    #[strum(serialize = "read-file")]
    #[strum(message = "读取文件", detailed_message = "read")]
    ReadFile,

    #[strum(serialize = "write-file")]
    #[strum(message = "写入文件", detailed_message = "write")]
    WriteFile,

    #[strum(serialize = "edit-file")]
    #[strum(message = "编辑文件", detailed_message = "edit")]
    EditFile,

    #[strum(serialize = "find-files")]
    #[strum(message = "查找文件", detailed_message = "find")]
    FindFiles,

    #[strum(serialize = "list-directory")]
    #[strum(message = "查找文件", detailed_message = "ls")]
    ListDirectory,

    #[strum(serialize = "grep")]
    #[strum(message = "搜索内容", detailed_message = "grep")]
    Grep,

    #[strum(serialize = "screenshot")]
    #[strum(message = "截取屏幕", detailed_message = "screenshot")]
    Screenshot,

    #[strum(serialize = "send-image")]
    #[strum(message = "发送图片", detailed_message = "send-image")]
    SendImage,

    #[strum(serialize = "send-email")]
    #[strum(message = "发送邮件", detailed_message = "email")]
    SendEmail,

    #[strum(serialize = "memory_search")]
    #[strum(message = "搜索记忆", detailed_message = "memory-search")]
    MemorySearch,

    #[strum(serialize = "memory_write")]
    #[strum(message = "写入记忆", detailed_message = "memory-write")]
    MemoryWrite,

    #[strum(serialize = "tape-handoff")]
    #[strum(message = "交接任务", detailed_message = "tape-handoff")]
    TapeHandoff,

    #[strum(serialize = "tape-info")]
    #[strum(message = "查看会话", detailed_message = "tape-info")]
    TapeInfo,

    #[strum(serialize = "user-note", serialize = "write-user-note")]
    #[strum(message = "记录笔记", detailed_message = "note")]
    UserNote,

    #[strum(serialize = "distill-user-notes")]
    #[strum(message = "整理笔记", detailed_message = "note-distill")]
    DistillUserNotes,

    #[strum(serialize = "settings")]
    #[strum(message = "调整设置", detailed_message = "settings")]
    Settings,

    #[strum(
        serialize = "composio_list",
        serialize = "composio_execute",
        serialize = "composio_connect",
        serialize = "composio_accounts"
    )]
    #[strum(message = "执行集成", detailed_message = "composio")]
    Composio,

    #[strum(serialize = "list-skills")]
    #[strum(message = "查看技能", detailed_message = "skills")]
    ListSkills,

    #[strum(serialize = "create-skill")]
    #[strum(message = "创建技能", detailed_message = "skill-create")]
    CreateSkill,

    #[strum(serialize = "delete-skill")]
    #[strum(message = "删除技能", detailed_message = "skill-delete")]
    DeleteSkill,

    #[strum(serialize = "install-mcp-server")]
    #[strum(message = "安装 MCP", detailed_message = "mcp-install")]
    InstallMcp,

    #[strum(serialize = "list-mcp-servers")]
    #[strum(message = "检查 MCP", detailed_message = "mcp-list")]
    ListMcp,

    #[strum(serialize = "remove-mcp-server")]
    #[strum(message = "移除 MCP", detailed_message = "mcp-remove")]
    RemoveMcp,

    #[strum(serialize = "dispatch-rara")]
    #[strum(message = "分派任务", detailed_message = "dispatch")]
    Dispatch,

    #[strum(serialize = "list-sessions")]
    #[strum(message = "查看会话列表", detailed_message = "sessions")]
    ListSessions,

    #[strum(serialize = "read-tape")]
    #[strum(message = "读取会话记录", detailed_message = "tape-read")]
    ReadTape,

    #[strum(serialize = "update-soul-state")]
    #[strum(message = "更新状态", detailed_message = "soul-update")]
    UpdateSoulState,

    #[strum(serialize = "evolve-soul")]
    #[strum(message = "自我进化", detailed_message = "soul-evolve")]
    EvolveSoul,
}

impl ToolKind {
    /// Parse a raw tool name, returning `None` for unknown tools.
    pub fn parse(raw: &str) -> Option<Self> { Self::from_str(raw).ok() }
}

/// Map raw tool names to shorter, human-friendly display names.
pub fn tool_display_name(raw: &str) -> &str {
    ToolKind::parse(raw)
        .and_then(|k| k.get_detailed_message())
        .unwrap_or(raw)
}

/// Map raw tool names to Chinese activity phrases for user-facing progress.
pub fn tool_activity_label(raw: &str) -> &str {
    ToolKind::parse(raw)
        .and_then(|k| k.get_message())
        .unwrap_or("处理中")
}

/// Extract a one-line summary from tool arguments based on the tool name.
///
/// Returns an empty string when no meaningful summary can be produced.
pub fn tool_arguments_summary(tool_name: &str, arguments: &serde_json::Value) -> String {
    let raw = match tool_name {
        "shell_execute" => arguments.get("command").and_then(|v| v.as_str()),
        "web_search" => arguments.get("query").and_then(|v| v.as_str()),
        "web_fetch" => arguments.get("url").and_then(|v| v.as_str()),
        "read-file" | "write-file" => arguments.get("path").and_then(|v| v.as_str()),
        _ => {
            // Try common field names, then fall back to the first string value.
            ["query", "command", "input", "path", "url"]
                .iter()
                .find_map(|key| arguments.get(*key).and_then(|v| v.as_str()))
                .or_else(|| first_string_value(arguments))
        }
    };

    match raw {
        Some(s) => truncate_summary(s, 80),
        None => String::new(),
    }
}

/// Return a `(display_name, full_summary)` pair with richer heuristics.
///
/// For `shell_execute` commands starting with `agent-browser`, the display name
/// becomes `"browser"` and the summary shows only the sub-command.  Shell noise
/// like trailing `2>&1` and pipe suffixes is stripped.  The summary is returned
/// **untruncated** — callers are responsible for truncating at the display
/// layer (e.g. via [`truncate_summary`]).
pub fn tool_display_info(tool_name: &str, arguments: &serde_json::Value) -> (String, String) {
    if tool_name == "shell_execute" {
        if let Some(cmd) = arguments.get("command").and_then(|v| v.as_str()) {
            let cleaned = clean_shell_command(cmd);
            if let Some(rest) = cleaned
                .strip_prefix("agent-browser ")
                .or_else(|| cleaned.strip_prefix("agent-browser\t"))
            {
                return ("browser".to_owned(), first_line(rest.trim()).to_owned());
            }
            return ("shell".to_owned(), first_line(&cleaned).to_owned());
        }
    }

    let name = tool_display_name(tool_name).to_owned();
    let raw = match tool_name {
        "bash" => arguments.get("command").and_then(|v| v.as_str()),
        "web_search" => arguments.get("query").and_then(|v| v.as_str()),
        "web_fetch" | "http-fetch" => arguments.get("url").and_then(|v| v.as_str()),
        "read-file" | "write-file" | "edit-file" => arguments.get("path").and_then(|v| v.as_str()),
        "find-files" | "list-directory" => arguments.get("path").and_then(|v| v.as_str()),
        "grep" => arguments.get("pattern").and_then(|v| v.as_str()),
        "memory_search" => arguments.get("query").and_then(|v| v.as_str()),
        "memory_write" => arguments.get("key").and_then(|v| v.as_str()),
        "tape-handoff" => arguments.get("summary").and_then(|v| v.as_str()),
        "tape-info" => arguments.get("session_id").and_then(|v| v.as_str()),
        "send-email" => arguments
            .get("subject")
            .and_then(|v| v.as_str())
            .or_else(|| arguments.get("to").and_then(|v| v.as_str())),
        "user-note" | "write-user-note" => arguments
            .get("title")
            .and_then(|v| v.as_str())
            .or_else(|| arguments.get("key").and_then(|v| v.as_str())),
        "dispatch-rara" => arguments.get("instruction").and_then(|v| v.as_str()),
        "read-tape" => arguments.get("session_id").and_then(|v| v.as_str()),
        "create-skill" | "delete-skill" => arguments.get("name").and_then(|v| v.as_str()),
        "settings" => arguments.get("action").and_then(|v| v.as_str()),
        _ => ["query", "command", "input", "path", "url", "name", "key"]
            .iter()
            .find_map(|key| arguments.get(*key).and_then(|v| v.as_str()))
            .or_else(|| first_string_value(arguments)),
    };

    let summary = raw.map(|s| first_line(s).to_owned()).unwrap_or_default();

    (name, summary)
}

/// Return the first line of `s` (everything before the first newline).
fn first_line(s: &str) -> &str { s.lines().next().unwrap_or(s) }

/// Strip common shell noise from a command string.
///
/// Removes trailing `2>&1` redirections and pipe suffixes like `| head -N`,
/// `| tail -N`, `| grep ...`.
fn clean_shell_command(cmd: &str) -> String {
    // Remove trailing 2>&1
    let s = cmd.trim();
    let s = s.strip_suffix("2>&1").unwrap_or(s).trim_end();

    // Remove trailing pipe segments: `| head ...`, `| tail ...`, `| grep ...`
    let mut result = s;
    loop {
        if let Some(pos) = result.rfind('|') {
            let after_pipe = result[pos + 1..].trim();
            if after_pipe.starts_with("head")
                || after_pipe.starts_with("tail")
                || after_pipe.starts_with("grep")
            {
                result = result[..pos].trim_end();
                continue;
            }
        }
        break;
    }

    result.to_owned()
}

/// Return the first string value found in a JSON object (top-level only).
fn first_string_value(value: &serde_json::Value) -> Option<&str> {
    value
        .as_object()
        .and_then(|obj| obj.values().find_map(|v| v.as_str()))
}

/// Take only the first line of `s`, truncating to `max_chars` with an ellipsis
/// if needed.
pub fn truncate_summary(s: &str, max_chars: usize) -> String {
    let first_line = s.lines().next().unwrap_or(s);
    let char_count = first_line.chars().count();
    if char_count <= max_chars {
        first_line.to_owned()
    } else {
        let truncated: String = first_line.chars().take(max_chars).collect();
        format!("{truncated}\u{2026}") // U+2026 = ...
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use strum::EnumMessage;

    use super::*;

    #[test]
    fn tool_kind_from_str() {
        assert_eq!(ToolKind::parse("bash"), Some(ToolKind::Bash));
        assert_eq!(
            ToolKind::parse("shell_execute"),
            Some(ToolKind::ShellExecute)
        );
        assert_eq!(ToolKind::parse("http-fetch"), Some(ToolKind::WebFetch));
        assert_eq!(ToolKind::parse("composio_list"), Some(ToolKind::Composio));
        assert_eq!(
            ToolKind::parse("composio_execute"),
            Some(ToolKind::Composio)
        );
        assert_eq!(ToolKind::parse("user-note"), Some(ToolKind::UserNote));
        assert_eq!(ToolKind::parse("write-user-note"), Some(ToolKind::UserNote));
        assert_eq!(ToolKind::parse("unknown_tool"), None);
    }

    #[test]
    fn tool_kind_messages() {
        assert_eq!(ToolKind::Bash.get_message(), Some("执行命令"));
        assert_eq!(ToolKind::Bash.get_detailed_message(), Some("bash"));
        assert_eq!(ToolKind::WebSearch.get_message(), Some("搜索网页"));
        assert_eq!(ToolKind::WebSearch.get_detailed_message(), Some("search"));
        assert_eq!(ToolKind::ListMcp.get_message(), Some("检查 MCP"));
        assert_eq!(ToolKind::ListMcp.get_detailed_message(), Some("mcp-list"));
    }

    #[test]
    fn activity_label_via_function() {
        assert_eq!(tool_activity_label("shell_execute"), "执行命令");
        assert_eq!(tool_activity_label("bash"), "执行命令");
        assert_eq!(tool_activity_label("web_search"), "搜索网页");
        assert_eq!(tool_activity_label("list-mcp-servers"), "检查 MCP");
        assert_eq!(tool_activity_label("remove-mcp-server"), "移除 MCP");
        assert_eq!(tool_activity_label("unknown_tool"), "处理中");
    }

    #[test]
    fn display_name_via_function() {
        assert_eq!(tool_display_name("shell_execute"), "shell");
        assert_eq!(tool_display_name("web_search"), "search");
        assert_eq!(tool_display_name("web_fetch"), "fetch");
        assert_eq!(tool_display_name("read-file"), "read");
        assert_eq!(tool_display_name("bash"), "bash");
        assert_eq!(tool_display_name("grep"), "grep");
        assert_eq!(tool_display_name("tape-handoff"), "tape-handoff");
        assert_eq!(tool_display_name("memory_search"), "memory-search");
        // Unknown tools fall back to raw name.
        assert_eq!(tool_display_name("some-custom-tool"), "some-custom-tool");
    }

    #[test]
    fn summary_shell() {
        let args = json!({"command": "ls -la /tmp"});
        assert_eq!(
            tool_arguments_summary("shell_execute", &args),
            "ls -la /tmp"
        );
    }

    #[test]
    fn summary_fallback_first_string() {
        let args = json!({"foo": "bar"});
        assert_eq!(tool_arguments_summary("unknown_tool", &args), "bar");
    }

    #[test]
    fn summary_empty_on_no_strings() {
        let args = json!({"count": 42});
        assert_eq!(tool_arguments_summary("unknown_tool", &args), "");
    }

    #[test]
    fn truncate_long_line() {
        let long = "a".repeat(100);
        let result = truncate_summary(&long, 80);
        assert_eq!(result.chars().count(), 81); // 80 + ellipsis
        assert!(result.ends_with('\u{2026}'));
    }

    #[test]
    fn truncate_multiline() {
        let s = "first line\nsecond line";
        assert_eq!(truncate_summary(s, 80), "first line");
    }

    #[test]
    fn clean_shell_strips_redirect() {
        assert_eq!(clean_shell_command("ls -la 2>&1"), "ls -la");
    }

    #[test]
    fn clean_shell_strips_pipes() {
        assert_eq!(
            clean_shell_command("cat file.txt | grep foo | head -20 2>&1"),
            "cat file.txt"
        );
    }

    #[test]
    fn clean_shell_preserves_meaningful_pipes() {
        assert_eq!(
            clean_shell_command("cat file.txt | sort | uniq"),
            "cat file.txt | sort | uniq"
        );
    }

    #[test]
    fn display_info_agent_browser() {
        let args = json!({"command": "agent-browser click @e1 2>&1"});
        let (name, summary) = tool_display_info("shell_execute", &args);
        assert_eq!(name, "browser");
        assert_eq!(summary, "click @e1");
    }

    #[test]
    fn display_info_regular_shell() {
        let args = json!({"command": "ls -la /tmp | head -5 2>&1"});
        let (name, summary) = tool_display_info("shell_execute", &args);
        assert_eq!(name, "shell");
        assert_eq!(summary, "ls -la /tmp");
    }

    #[test]
    fn display_info_non_shell() {
        let args = json!({"query": "rust async"});
        let (name, summary) = tool_display_info("web_search", &args);
        assert_eq!(name, "search");
        assert_eq!(summary, "rust async");
    }
}
