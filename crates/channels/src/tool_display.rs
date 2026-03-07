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
        "web_search" => "search",
        "web_fetch" => "fetch",
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
        "read_file" | "write_file" => arguments.get("path").and_then(|v| v.as_str()),
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
    use super::*;
    use serde_json::json;

    #[test]
    fn display_name_mapping() {
        assert_eq!(tool_display_name("shell_execute"), "shell");
        assert_eq!(tool_display_name("web_search"), "search");
        assert_eq!(tool_display_name("web_fetch"), "fetch");
        assert_eq!(tool_display_name("read_file"), "read_file");
    }

    #[test]
    fn summary_shell() {
        let args = json!({"command": "ls -la /tmp"});
        assert_eq!(tool_arguments_summary("shell_execute", &args), "ls -la /tmp");
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
}
