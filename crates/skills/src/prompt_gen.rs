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

//! System prompt generation for LLM skill injection.
//!
//! Generates an `<available_skills>` XML block listing all discovered skills
//! with their names, sources, paths, and descriptions, suitable for injection
//! into the LLM system prompt.

use crate::types::SkillMetadata;

/// Maximum one-line description length before truncation.
const MAX_SHORT_DESC_CHARS: usize = 80;

/// Truncate a string to at most `max_chars` characters, appending "…" if cut.
fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let end = s
        .char_indices()
        .nth(max_chars)
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    format!("{}…", &s[..end])
}

/// Escape XML special characters to prevent prompt injection via skill
/// metadata.
fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Extract the first sentence from a description.
///
/// Looks for English (". ") or Chinese ("。") sentence boundaries. If none
/// found, truncates to [`MAX_SHORT_DESC_CHARS`] with an ellipsis.
fn first_sentence(s: &str) -> String {
    // English sentence boundary: period followed by space
    if let Some(idx) = s.find(". ") {
        return s[..=idx].to_string();
    }
    // Chinese sentence boundary
    if let Some(idx) = s.find('。') {
        let end = idx + '。'.len_utf8();
        return s[..end].to_string();
    }
    // No sentence boundary — truncate if needed
    truncate(s, MAX_SHORT_DESC_CHARS)
}

/// Generate a compact skills listing for injection into the system prompt.
///
/// Each skill is rendered as a single line (`- name: short_description`) to
/// minimize token usage (~800 tokens for 25 skills vs ~6,250 with full XML).
pub fn generate_skills_prompt(skills: &[SkillMetadata]) -> String {
    if skills.is_empty() {
        return String::new();
    }

    // Sort by name for deterministic output (improves provider-side prompt cache
    // hit rate by keeping the prefix stable across restarts and rehashes).
    let mut sorted: Vec<&SkillMetadata> = skills.iter().collect();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));

    let mut out = String::from("## Available Skills\n\n");
    for skill in &sorted {
        let short_desc = first_sentence(&skill.description);
        out.push_str(&format!("- {}: {}\n", escape_xml(&skill.name), short_desc));
    }
    out.push('\n');
    out.push_str(
        "To use a skill, read its SKILL.md file for full instructions.\nUse YOUR actual tools \
         (http_fetch, bash, read_file, etc.) — not tool names from skills written for other \
         environments.\n\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_multibyte() {
        let input = "你好世界测试文本额外内容";
        let result = truncate(input, 5);
        assert_eq!(result, "你好世界测…");
        assert_eq!(truncate(input, 50).chars().count(), input.chars().count());
    }

    #[test]
    fn first_sentence_english() {
        assert_eq!(
            first_sentence("This is a tool. It does things."),
            "This is a tool."
        );
    }

    #[test]
    fn first_sentence_chinese() {
        assert_eq!(first_sentence("这是一个工具。它做事情。"), "这是一个工具。");
    }

    #[test]
    fn first_sentence_no_period_short() {
        assert_eq!(first_sentence("Short text"), "Short text");
    }

    #[test]
    fn first_sentence_no_period_long() {
        let long = "a".repeat(100);
        let result = first_sentence(&long);
        assert_eq!(result.chars().count(), MAX_SHORT_DESC_CHARS + 1); // 80 + "…"
        assert!(result.ends_with('…'));
    }
}
