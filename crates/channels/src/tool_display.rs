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

/// Map raw tool names to shorter, human-friendly display names.
pub fn tool_display_name(raw: &str) -> &str {
    match raw {
        "shell_execute" => "shell",
        "bash" => "bash",
        "web_search" => "search",
        "web_fetch" | "http-fetch" => "fetch",
        "read-file" => "read",
        "write-file" => "write",
        "edit-file" => "edit",
        "find-files" => "find",
        "list-directory" => "ls",
        "grep" => "grep",
        "screenshot" => "screenshot",
        "send-image" => "send-image",
        "send-email" => "email",
        "memory_search" => "memory-search",
        "memory_write" => "memory-write",
        "tape-handoff" => "tape-handoff",
        "tape-info" => "tape-info",
        "user-note" => "note",
        "settings" => "settings",
        "composio" => "composio",
        "list-skills" => "skills",
        "create-skill" => "skill-create",
        "delete-skill" => "skill-delete",
        "install-mcp-server" => "mcp-install",
        "list-mcp-servers" => "mcp-list",
        "remove-mcp-server" => "mcp-remove",
        "dispatch-rara" => "dispatch",
        "list-sessions" => "sessions",
        "read-tape" => "tape-read",
        "write-user-note" => "note-write",
        "distill-user-notes" => "note-distill",
        "update-soul-state" => "soul-update",
        "evolve-soul" => "soul-evolve",
        other => other,
    }
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

/// Return a `(display_name, summary)` pair with richer heuristics.
///
/// For `shell_execute` commands starting with `agent-browser`, the display name
/// becomes `"browser"` and the summary shows only the sub-command.  Shell noise
/// like trailing `2>&1` and pipe suffixes is stripped.  The summary is
/// truncated to 60 characters (suitable for Telegram's narrower viewport).
pub fn tool_display_info(tool_name: &str, arguments: &serde_json::Value) -> (String, String) {
    if tool_name == "shell_execute" {
        if let Some(cmd) = arguments.get("command").and_then(|v| v.as_str()) {
            let cleaned = clean_shell_command(cmd);
            if let Some(rest) = cleaned
                .strip_prefix("agent-browser ")
                .or_else(|| cleaned.strip_prefix("agent-browser\t"))
            {
                return ("browser".to_owned(), truncate_summary(rest.trim(), 60));
            }
            return ("shell".to_owned(), truncate_summary(&cleaned, 60));
        }
    }

    let name = tool_display_name(tool_name).to_owned();
    let raw = match tool_name {
        "bash" => arguments.get("command").and_then(|v| v.as_str()),
        "web_search" => arguments.get("query").and_then(|v| v.as_str()),
        "web_fetch" | "http-fetch" => arguments.get("url").and_then(|v| v.as_str()),
        "read-file" | "write-file" | "edit-file" => {
            arguments.get("path").and_then(|v| v.as_str())
        }
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

    let summary = match raw {
        Some(s) => truncate_summary(s, 80),
        None => String::new(),
    };

    (name, summary)
}

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
fn truncate_summary(s: &str, max_chars: usize) -> String {
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

    use super::*;

    #[test]
    fn display_name_mapping() {
        assert_eq!(tool_display_name("shell_execute"), "shell");
        assert_eq!(tool_display_name("web_search"), "search");
        assert_eq!(tool_display_name("web_fetch"), "fetch");
        assert_eq!(tool_display_name("read-file"), "read");
        assert_eq!(tool_display_name("bash"), "bash");
        assert_eq!(tool_display_name("grep"), "grep");
        assert_eq!(tool_display_name("tape-handoff"), "tape-handoff");
        assert_eq!(tool_display_name("memory_search"), "memory-search");
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
