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

//! Runtime detection of lazy LLM "ack" responses.
//!
//! When the LLM produces a planning message ("I'll look into this...",
//! "让我检查一下...") instead of calling tools, the agent loop injects
//! a nudge forcing the model to act. The ack message is kept in context
//! (marked as intermediate) so the model sees its own plan.
//!
//! Pattern sources: hermes-agent, Logos (GregsGreyCode), Mullet (tomgould).

use std::sync::OnceLock;

use regex::Regex;

use crate::llm;

/// Maximum assistant response length (chars) to consider.
/// Longer responses are likely substantive, not lazy acks.
/// Aligned with hermes (1200 chars).
const MAX_ACK_LENGTH_CHARS: usize = 1200;

/// Compiled regex for English future-ack phrases with word boundaries.
/// Sources: hermes original + Logos (`i'm going to`) + Mullet (`i need to`,
/// `i should`, `allow me to`). `let's` is checked via substring (below)
/// because the apostrophe breaks `\b` matching.
fn future_ack_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\b(i[''\u{2019}]ll|i will|let me|i can do that|i can help with that|i[''\u{2019}]m going to|i need to|i should|allow me to)\b",
        )
        .expect("ack regex must compile")
    })
}

/// English ack phrases that contain apostrophes and can't use `\b` reliably.
const ENGLISH_ACK_SUBSTRINGS: &[&str] = &["let's", "let\u{2019}s"];

/// Chinese future-ack phrases (no word boundaries needed — CJK has no spaces).
const CHINESE_ACK_PATTERNS: &[&str] = &[
    "让我",
    "我来",
    "我去",
    "我帮你",
    "好的，我",
    "我先",
    "我看看",
    "我查一下",
    "我下一步",
    "我接下来",
    "下一步我",
    "接下来我",
];

/// Action verbs confirming described future work.
/// Sources: hermes (19) + Logos (`try`, `attempt`) + Mullet + common LLM
/// patterns.
const ACTION_MARKERS: &[&str] = &[
    // hermes original
    "look into",
    "look at",
    "inspect",
    "scan",
    "check",
    "analyz",
    "review",
    "explore",
    "read",
    "open",
    "run",
    "test",
    "fix",
    "debug",
    "search",
    "find",
    "walkthrough",
    "report back",
    "summarize",
    // Logos additions
    "try",
    "attempt",
    "investigate",
    "examine",
    // Common LLM output patterns
    "modify",
    "update",
    "create",
    "write",
    "implement",
    "diagnose",
    "verify",
    "build",
    "execute",
    "set up",
    "trace",
    "refactor",
    "compile",
    // Chinese — rara extension
    "查看",
    "查实",
    "检查",
    "确认",
    "分析",
    "调试",
    "搜索",
    "修复",
    "测试",
    "编写",
    "构建",
    "实现",
    "排查",
    "验证",
];

/// Result phrases that indicate the model is giving an answer, not planning.
/// Adapted from Mullet's result_phrases — if any of these appear, the
/// response is genuine even if it also contains ack/action words.
const RESULT_PHRASES: &[&str] = &[
    // English
    "here is",
    "here are",
    "the answer",
    "the result",
    "i found",
    "it shows",
    "the output is",
    "the error is",
    "the issue is",
    "the problem is",
    "the fix is",
    // Chinese
    "结果是",
    "问题是",
    "原因是",
    "找到了",
    "已经",
    "完成了",
    "成功了",
    "错误是",
];

/// Strip `<think>...</think>` blocks from assistant text before detection.
/// Aligned with hermes `_strip_think_blocks` — reasoning content should
/// not trigger ack detection.
fn strip_think_blocks(text: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?s)<think>.*?</think>").expect("think-strip regex must compile")
    });
    re.replace_all(text, "").to_string()
}

/// Check whether an assistant response is an intermediate ack that should
/// be nudged instead of ending the turn.
///
/// Based on hermes-agent `_looks_like_codex_intermediate_ack`, with patterns
/// expanded from Logos and Mullet forks, and adapted for rara:
///
/// 1. Strip `<think>` blocks (reasoning shouldn't trigger detection).
/// 2. If the last message is a tool result, the model is summarizing tool
///    output — that's a genuine answer, not laziness.
/// 3. If response contains result phrases ("here is", "i found"), it's a
///    genuine answer even if it also mentions future actions.
/// 4. Short text (≤1200 chars) with a future-tense phrase (word-boundary
///    matched for English, substring for Chinese) + action verb = lazy ack.
///
/// Workspace markers from hermes are omitted (rara is always in workspace).
pub fn looks_like_intermediate_ack(assistant_text: &str, messages: &[llm::Message]) -> bool {
    // If the last message is a tool result, the model is responding to tool
    // output — that's a genuine summary, not laziness. Skip detection.
    if let Some(last) = messages.last() {
        if matches!(last.role, llm::Role::Tool) {
            return false;
        }
    }

    // Strip think blocks, then check length.
    let stripped = strip_think_blocks(assistant_text);
    let text = stripped.trim();
    if text.is_empty() || text.chars().count() > MAX_ACK_LENGTH_CHARS {
        return false;
    }

    let lower = text.to_lowercase();

    // Result phrase exclusion (Mullet): if the response already contains
    // result indicators, it's a genuine answer — not a lazy ack.
    if RESULT_PHRASES.iter().any(|rp| lower.contains(rp)) {
        return false;
    }

    // Future-ack detection: word-boundary regex for English, substring for Chinese.
    let has_future_ack = future_ack_regex().is_match(&lower)
        || ENGLISH_ACK_SUBSTRINGS.iter().any(|p| lower.contains(p))
        || CHINESE_ACK_PATTERNS.iter().any(|p| lower.contains(p));
    if !has_future_ack {
        return false;
    }

    // Action marker: confirms the model is describing future work.
    ACTION_MARKERS.iter().any(|marker| lower.contains(marker))
}

/// Nudge message injected after an ack is detected.
pub const ACK_NUDGE_MESSAGE: &str = "[System: Continue now. Execute the required tool calls and \
                                     only send your final answer after completing the task.]";

/// Maximum ack nudges per turn (hermes: `codex_ack_continuations < 2`).
pub const MAX_ACK_NUDGES: usize = 2;

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_messages() -> Vec<llm::Message> { vec![] }

    fn messages_ending_with_tool_result() -> Vec<llm::Message> {
        vec![
            llm::Message::user("hello".to_string()),
            llm::Message::tool_result("call_1", "result"),
        ]
    }

    fn messages_with_tools_then_user() -> Vec<llm::Message> {
        vec![
            llm::Message::user("hello".to_string()),
            llm::Message::tool_result("call_1", "result"),
            llm::Message::assistant("I found the file.".to_string()),
            llm::Message::user("ok now fix it".to_string()),
        ]
    }

    // --- Positive detections ---

    #[test]
    fn detects_english_planning_ack() {
        assert!(looks_like_intermediate_ack(
            "I'll look into the build failure and check the logs.",
            &empty_messages(),
        ));
    }

    #[test]
    fn detects_chinese_planning_ack() {
        assert!(looks_like_intermediate_ack(
            "让我检查一下构建日志",
            &empty_messages()
        ));
    }

    #[test]
    fn detects_ack_after_tools_when_last_msg_is_user() {
        assert!(looks_like_intermediate_ack(
            "I'll look into the build failure and check the logs.",
            &messages_with_tools_then_user(),
        ));
    }

    #[test]
    fn detects_hermes_style_ack() {
        assert!(looks_like_intermediate_ack(
            "I can help with that. Let me search the codebase and report back.",
            &empty_messages(),
        ));
    }

    #[test]
    fn detects_polite_ack() {
        assert!(looks_like_intermediate_ack(
            "好的，我来帮你查看一下这个问题",
            &empty_messages(),
        ));
    }

    #[test]
    fn detects_next_step_chinese_ack() {
        assert!(looks_like_intermediate_ack(
            "我下一步会直接从这些官方文档里把触发机制查实",
            &empty_messages(),
        ));
    }

    #[test]
    fn detects_logos_going_to_pattern() {
        assert!(looks_like_intermediate_ack(
            "I'm going to investigate the test failures.",
            &empty_messages(),
        ));
    }

    #[test]
    fn detects_mullet_need_to_pattern() {
        assert!(looks_like_intermediate_ack(
            "I need to check the configuration file first.",
            &empty_messages(),
        ));
    }

    #[test]
    fn detects_lets_pattern() {
        assert!(looks_like_intermediate_ack(
            "Let's examine the build logs and trace the deployment.",
            &empty_messages(),
        ));
    }

    #[test]
    fn strip_think_preserves_visible_ack() {
        assert!(looks_like_intermediate_ack(
            "<think>reasoning here</think>Let me check the build logs.",
            &empty_messages(),
        ));
    }

    // --- Negative detections (should NOT trigger) ---

    #[test]
    fn ignores_when_last_msg_is_tool_result() {
        assert!(!looks_like_intermediate_ack(
            "I'll look into the build failure.",
            &messages_ending_with_tool_result(),
        ));
    }

    #[test]
    fn ignores_long_substantive_response() {
        let long_text = "I'll analyze this. ".repeat(200);
        assert!(!looks_like_intermediate_ack(&long_text, &empty_messages()));
    }

    #[test]
    fn ignores_genuine_answer() {
        assert!(!looks_like_intermediate_ack(
            "The build succeeded. All 42 tests passed.",
            &empty_messages(),
        ));
    }

    #[test]
    fn ignores_empty() {
        assert!(!looks_like_intermediate_ack("", &empty_messages()));
    }

    #[test]
    fn word_boundary_no_false_positive() {
        assert!(!looks_like_intermediate_ack(
            "I filled the test report and checked the results.",
            &empty_messages(),
        ));
    }

    #[test]
    fn ignores_ack_inside_think_block() {
        assert!(!looks_like_intermediate_ack(
            "<think>I'll look into this and check the logs.</think>The answer is 42.",
            &empty_messages(),
        ));
    }

    // Result phrase exclusion (Mullet)
    #[test]
    fn ignores_response_with_result_phrase() {
        assert!(!looks_like_intermediate_ack(
            "I'll summarize what I found. Here is the configuration issue.",
            &empty_messages(),
        ));
    }

    #[test]
    fn ignores_chinese_result_response() {
        assert!(!looks_like_intermediate_ack(
            "让我总结一下，问题是配置文件缺少必要字段",
            &empty_messages(),
        ));
    }
}
