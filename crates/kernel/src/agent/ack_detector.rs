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
//! Aligned with hermes-agent `_looks_like_codex_intermediate_ack`.

use crate::llm;

/// Maximum assistant response length (bytes) to consider.
/// Longer responses are likely substantive, not lazy acks.
/// hermes uses 1200 chars; we use 2000 bytes to cover CJK
/// (3 bytes/char × ~650 chars ≈ 2000 bytes).
const MAX_ACK_LENGTH_BYTES: usize = 2000;

/// Future-tense phrases signaling the model is *planning* but hasn't acted.
const FUTURE_ACK_PATTERNS: &[&str] = &[
    // English — aligned with hermes regex
    "i'll",
    "i\u{2019}ll", // curly apostrophe
    "i will",
    "let me",
    "i can do that",
    "i can help with that",
    // Chinese — rara extension
    "让我",
    "我来",
    "我去",
    "我帮你",
    "好的，我",
    "我先",
    "我看看",
    "我查一下",
];

/// Action verbs confirming described future work. Aligned with hermes.
const ACTION_MARKERS: &[&str] = &[
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
    "investigate",
    "examine",
    // Chinese — rara extension
    "查看",
    "检查",
    "分析",
    "调试",
    "搜索",
    "修复",
    "测试",
];

/// Check whether an assistant response is an intermediate ack that should
/// be nudged instead of ending the turn.
///
/// Mirrors hermes-agent logic:
/// 1. If the conversation already contains tool results, the model has started
///    working — a text-only follow-up is a genuine answer.
/// 2. Short text with a future-tense phrase + action verb = lazy ack.
///
/// Workspace markers from hermes are omitted because rara always operates
/// in a workspace context (personal agent, not general chat).
pub fn looks_like_intermediate_ack(assistant_text: &str, messages: &[llm::Message]) -> bool {
    // If any tool results exist in the conversation, the model already
    // took action — don't nudge a genuine summary. Aligned with hermes:
    // `if any(msg.get("role") == "tool" for msg in messages): return False`
    if messages.iter().any(|m| matches!(m.role, llm::Role::Tool)) {
        return false;
    }

    let text = assistant_text.trim();
    if text.is_empty() || text.len() > MAX_ACK_LENGTH_BYTES {
        return false;
    }

    let lower = text.to_lowercase();

    let has_future_ack = FUTURE_ACK_PATTERNS.iter().any(|pat| lower.contains(pat));
    if !has_future_ack {
        return false;
    }

    ACTION_MARKERS.iter().any(|marker| lower.contains(marker))
}

/// Nudge message injected after an ack is detected.
/// Aligned with hermes: `"[System: Continue now. Execute the required tool
/// calls and only send your final answer after completing the task.]"`
pub const ACK_NUDGE_MESSAGE: &str = "[System: Continue now. Execute the required tool calls and \
                                     only send your final answer after completing the task.]";

/// Maximum ack nudges per turn (hermes: `codex_ack_continuations < 2`).
pub const MAX_ACK_NUDGES: usize = 2;

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_messages() -> Vec<llm::Message> { vec![] }

    fn messages_with_tool_result() -> Vec<llm::Message> {
        vec![
            llm::Message::user("hello".to_string()),
            llm::Message::tool_result("call_1", "result"),
        ]
    }

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
            &empty_messages(),
        ));
    }

    #[test]
    fn ignores_when_tools_already_called() {
        assert!(!looks_like_intermediate_ack(
            "I'll look into the build failure.",
            &messages_with_tool_result(),
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
    fn detects_polite_ack() {
        assert!(looks_like_intermediate_ack(
            "好的，我来帮你查看一下这个问题",
            &empty_messages(),
        ));
    }

    #[test]
    fn detects_hermes_style_ack() {
        assert!(looks_like_intermediate_ack(
            "I can help with that. Let me search the codebase and report back.",
            &empty_messages(),
        ));
    }
}
