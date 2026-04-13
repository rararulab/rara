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

//! Runtime detection of lazy LLM responses.
//!
//! Three detection tiers (inspired by Triggerfish's quality classifier):
//!
//! 1. **Short ack** (≤ 2000 chars) — full-text pattern match against 5 laziness
//!    categories: future-planning, permission-seeking, self-narration,
//!    deferral, and conditional offering.
//! 2. **Trailing intent** (2000–8000 chars) — tail-only check (last 600 chars)
//!    for lazy patterns. Catches GPT's verbose analyses that end with "if you
//!    want, I can..." offers.
//! 3. **Dense narration** (2000–8000 chars) — 5+ intent phrases in a single
//!    response = planning essay regardless of where they appear.
//!
//! All tiers share guards: tool-result skip, think-block stripping, and
//! result-phrase exclusion.
//!
//! Pattern sources: hermes-agent, Logos, Mullet, Triggerfish, Omegon,
//! aider, Cursor, SLOP_Detector, stop-slop, and real-world rara logs.

use std::sync::OnceLock;

use regex::Regex;

use crate::llm;

/// Maximum assistant response length (chars) for short-ack detection.
/// GPT models produce verbose planning responses that exceed 1200 chars.
const MAX_ACK_LENGTH_CHARS: usize = 2000;

/// Tail window (chars) for verbose narration detection.
/// GPT writes long analyses ending with "if you want, I can..." offers;
/// checking only the tail avoids false positives from incidental matches.
const TAIL_CHECK_CHARS: usize = 600;

/// Upper bound for verbose narration tier. Responses beyond this are
/// assumed to be genuine long-form output (documentation, reports).
const MAX_VERBOSE_NARRATION_CHARS: usize = 8000;

/// Minimum intent-phrase count to trigger dense-narration detection.
/// Aligned with Triggerfish's `DENSE_NARRATION_THRESHOLD`.
const DENSE_NARRATION_THRESHOLD: usize = 5;

// ---------------------------------------------------------------------------
// Category 1: Future-tense planning ("I'll...", "Let me...")
// Sources: hermes, Logos, Mullet
// ---------------------------------------------------------------------------

/// English future-ack regex with word boundaries.
fn future_ack_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(concat!(
            r"(?i)\b(",
            // hermes original
            r"i[''\u{2019}]ll",
            r"|i will",
            r"|let me",
            r"|i can do that",
            r"|i can help with that",
            // Logos
            r"|i[''\u{2019}]m going to",
            // Mullet
            r"|i need to",
            r"|i should",
            r"|allow me to",
            // Common LLM patterns
            r"|i[''\u{2019}]d like to",
            r"|i[''\u{2019}]d want to",
            r"|to start,? i",
            r"|first,? i",
            r")\b",
        ))
        .expect("future_ack regex must compile")
    })
}

// ---------------------------------------------------------------------------
// Category 2: Permission seeking ("Should I...", "Would you like me to...")
// Sources: Cursor anti-patterns, real-world rara/GPT logs
// ---------------------------------------------------------------------------

fn permission_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(concat!(
            r"(?i)\b(",
            r"would you like me to",
            r"|shall i",
            r"|should i",
            r"|do you want me to",
            r"|want me to",
            r"|if you[''\u{2019}]d like,? i",
            r"|if you want,? i",
            r"|i can .{0,30} if you",
            r")\b",
        ))
        .expect("permission regex must compile")
    })
}

// ---------------------------------------------------------------------------
// Category 3: Self-narration / plan description
// Sources: Omegon "stop narrating", Cursor, real-world logs
// ---------------------------------------------------------------------------

fn narration_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(concat!(
            r"(?i)\b(",
            r"here[''\u{2019}]s my plan",
            r"|here[''\u{2019}]s what i[''\u{2019}]ll do",
            r"|here[''\u{2019}]s my approach",
            r"|my plan is to",
            r"|my approach (is|will be) to",
            r"|the strategy (is|would be) to",
            r"|the approach (is|would be) to",
            r"|the next step (is|would be) to",
            r"|i[''\u{2019}]m planning to",
            r"|i plan to",
            r")\b",
        ))
        .expect("narration regex must compile")
    })
}

/// English ack phrases that contain apostrophes (can't use `\b` reliably).
const ENGLISH_ACK_SUBSTRINGS: &[&str] = &["let's", "let\u{2019}s"];

/// Chinese lazy patterns — all categories combined.
/// No word boundaries needed for CJK.
const CHINESE_LAZY_PATTERNS: &[&str] = &[
    // Category 1: Future planning
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
    "我打算",
    "我准备",
    "我会去",
    "首先我",
    // Category 2: Permission seeking
    "要不要我",
    "需要我",
    "要我帮你",
    "你要我",
    "你希望我",
    "你需要我",
    // Category 3: Self-narration
    "我的方案是",
    "我的思路是",
    "我的计划是",
    "具体步骤",
    "方案如下",
    "思路如下",
    "计划如下",
    "步骤如下",
    // Category 4: Deferral
    "之后我",
    "然后我",
    "接着我",
    "等一下我",
    "后续我",
    // Category 5: Conditional offering — GPT's verbose "if you want, I can..."
    "如果你要",
    "如果你愿意",
    "如果你需要",
    "你看怎么样",
    "你觉得呢",
];

// ---------------------------------------------------------------------------
// Action markers — confirms the model is describing work, not giving results
// ---------------------------------------------------------------------------

/// Action verbs. Sources: hermes (19) + Logos (2) + Mullet + common LLM + CJK.
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
    // Logos
    "try",
    "attempt",
    "investigate",
    "examine",
    // Common LLM patterns
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
    "deploy",
    "configure",
    "install",
    "migrate",
    "resolve",
    "troubleshoot",
    "address",
    "handle",
    "patch",
    "adjust",
    "optimize",
    "clean up",
    "restructure",
    "rewrite",
    // Chinese
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
    "部署",
    "配置",
    "迁移",
    "优化",
    "重构",
    "处理",
    "解决",
    "调整",
    "梳理",
    "整理",
];

// ---------------------------------------------------------------------------
// Result phrases — genuine answers, NOT laziness (Mullet pattern)
// ---------------------------------------------------------------------------

const RESULT_PHRASES: &[&str] = &[
    // English
    "here is",
    "here are",
    "the answer is",
    "the result is",
    "i found that",
    "i found the",
    "it shows",
    "the output is",
    "the error is",
    "the issue is",
    "the problem is",
    "the fix is",
    "the root cause",
    "this is because",
    "this happens because",
    "the reason is",
    "i've completed",
    "i've finished",
    "i've fixed",
    "i've updated",
    "done.",
    "done!",
    "all set",
    "successfully",
    // Chinese
    "结果是",
    "问题是",
    "原因是",
    "找到了",
    "已经完成",
    "已经修复",
    "已经更新",
    "完成了",
    "成功了",
    "搞定了",
    "错误是",
    "根因是",
    "这是因为",
    "弄好了",
];

// ---------------------------------------------------------------------------
// Think-block stripping
// ---------------------------------------------------------------------------

fn strip_think_blocks(text: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?s)<think>.*?</think>").expect("think-strip regex must compile")
    });
    re.replace_all(text, "").to_string()
}

// ---------------------------------------------------------------------------
// Shared matchers — used by both short-ack and verbose-narration tiers
// ---------------------------------------------------------------------------

/// Whether lowercased text contains any lazy response pattern.
fn matches_lazy_pattern(lower: &str) -> bool {
    future_ack_regex().is_match(lower)
        || permission_regex().is_match(lower)
        || narration_regex().is_match(lower)
        || ENGLISH_ACK_SUBSTRINGS.iter().any(|p| lower.contains(p))
        || CHINESE_LAZY_PATTERNS.iter().any(|p| lower.contains(p))
}

/// Whether lowercased text mentions a concrete action verb.
fn matches_action_marker(lower: &str) -> bool {
    ACTION_MARKERS.iter().any(|marker| lower.contains(marker))
}

/// Whether lowercased text contains phrases indicating genuine results.
fn matches_result_phrase(lower: &str) -> bool { RESULT_PHRASES.iter().any(|rp| lower.contains(rp)) }

/// Extract the last `n` characters of a string.
fn tail_chars(text: &str, n: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= n {
        return text.to_string();
    }
    chars[chars.len() - n..].iter().collect()
}

/// Count total intent-phrase occurrences across all laziness categories.
/// Used by the dense-narration tier — 5+ hits = planning essay.
fn count_intent_phrases(lower: &str) -> usize {
    let mut count = 0;
    count += future_ack_regex().find_iter(lower).count();
    count += permission_regex().find_iter(lower).count();
    count += narration_regex().find_iter(lower).count();
    for p in ENGLISH_ACK_SUBSTRINGS {
        count += lower.matches(p).count();
    }
    for p in CHINESE_LAZY_PATTERNS {
        count += lower.matches(p).count();
    }
    count
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Classification of detected laziness for differentiated nudging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AckKind {
    /// Short planning ack (≤ 2000 chars), full-text match.
    ShortAck,
    /// Long response with lazy tail (2000–8000 chars).
    TrailingIntent,
    /// Response saturated with 5+ intent phrases.
    DenseNarration,
}

impl AckKind {
    /// Nudge message appropriate for this laziness category.
    pub fn nudge_message(self) -> &'static str {
        match self {
            Self::ShortAck => SHORT_ACK_NUDGE,
            Self::TrailingIntent => TRAILING_INTENT_NUDGE,
            Self::DenseNarration => DENSE_NARRATION_NUDGE,
        }
    }
}

const SHORT_ACK_NUDGE: &str = "[System: You described what you intend to do but did not call any \
                               tools. Use your tools now to complete the task. Do not narrate — \
                               act.]";

const TRAILING_INTENT_NUDGE: &str = "[System: You wrote a lengthy analysis but ended with an \
                                     unfulfilled intention instead of executing. Stop discussing \
                                     — call a tool now to make concrete progress.]";

const DENSE_NARRATION_NUDGE: &str = "[System: Your response is a planning essay with multiple \
                                     stated intentions but zero tool calls. Stop narrating your \
                                     plan and execute it. Call a tool in your very next response.]";

/// Three-tier detection of lazy ack/hedge/narration that should be nudged
/// instead of ending the turn.
///
/// Returns the detected [`AckKind`] or `None` if the response is genuine.
/// See module docs for tier descriptions.
pub fn detect(assistant_text: &str, messages: &[llm::Message]) -> Option<AckKind> {
    // Guard: skip if last message is a tool result (genuine summary).
    if let Some(last) = messages.last() {
        if matches!(last.role, llm::Role::Tool) {
            return None;
        }
    }

    let stripped = strip_think_blocks(assistant_text);
    let text = stripped.trim();
    if text.is_empty() {
        return None;
    }

    let char_count = text.chars().count();
    let lower = text.to_lowercase();

    // Result phrases anywhere → genuine answer, skip all tiers.
    if matches_result_phrase(&lower) {
        return None;
    }

    // Upper bound: responses beyond MAX_VERBOSE_NARRATION_CHARS are assumed
    // to be genuine long-form output (documentation, reports).
    if char_count > MAX_VERBOSE_NARRATION_CHARS {
        return None;
    }

    // Tier 1: Short ack — full-text lazy pattern + action marker.
    if char_count <= MAX_ACK_LENGTH_CHARS {
        if matches_lazy_pattern(&lower) && matches_action_marker(&lower) {
            return Some(AckKind::ShortAck);
        }
        return None;
    }

    // For longer responses (2000–8000 chars), two detection strategies:

    // Tier 3: Dense narration — many intent phrases across the full text.
    // Checked first because it is higher confidence than tail-only check.
    if count_intent_phrases(&lower) >= DENSE_NARRATION_THRESHOLD {
        return Some(AckKind::DenseNarration);
    }

    // Tier 2: Trailing intent — lazy pattern in the tail only.
    // GPT writes long analyses ending with "if you want, I can..." offers.
    let tail = tail_chars(&lower, TAIL_CHECK_CHARS);
    if matches_lazy_pattern(&tail) && matches_action_marker(&tail) {
        return Some(AckKind::TrailingIntent);
    }

    None
}

/// Convenience wrapper — returns `true` if any laziness tier matches.
pub fn looks_like_intermediate_ack(assistant_text: &str, messages: &[llm::Message]) -> bool {
    detect(assistant_text, messages).is_some()
}

/// Maximum nudges per turn.
pub const MAX_ACK_NUDGES: usize = 2;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn empty() -> Vec<llm::Message> { vec![] }

    fn after_tool() -> Vec<llm::Message> {
        vec![
            llm::Message::user("hello".to_string()),
            llm::Message::tool_result("c1", "result"),
        ]
    }

    fn after_user() -> Vec<llm::Message> {
        vec![
            llm::Message::user("hello".to_string()),
            llm::Message::tool_result("c1", "result"),
            llm::Message::assistant("I found the file.".to_string()),
            llm::Message::user("ok now fix it".to_string()),
        ]
    }

    // ── Category 1: Future-tense planning ──

    #[test]
    fn hermes_ill_look() {
        assert!(looks_like_intermediate_ack(
            "I'll look into the build failure and check the logs.",
            &empty(),
        ));
    }

    #[test]
    fn hermes_let_me() {
        assert!(looks_like_intermediate_ack(
            "I can help with that. Let me search the codebase and report back.",
            &empty(),
        ));
    }

    #[test]
    fn logos_going_to() {
        assert!(looks_like_intermediate_ack(
            "I'm going to investigate the test failures.",
            &empty(),
        ));
    }

    #[test]
    fn mullet_need_to() {
        assert!(looks_like_intermediate_ack(
            "I need to check the configuration file first.",
            &empty(),
        ));
    }

    #[test]
    fn mullet_lets() {
        assert!(looks_like_intermediate_ack(
            "Let's examine the build logs and trace the deployment.",
            &empty(),
        ));
    }

    #[test]
    fn id_like_to() {
        assert!(looks_like_intermediate_ack(
            "I'd like to review the database migrations first.",
            &empty(),
        ));
    }

    #[test]
    fn chinese_let_me() {
        assert!(looks_like_intermediate_ack(
            "让我检查一下构建日志",
            &empty()
        ));
    }

    #[test]
    fn chinese_next_step() {
        assert!(looks_like_intermediate_ack(
            "我下一步会直接从这些官方文档里把触发机制查实",
            &empty(),
        ));
    }

    #[test]
    fn chinese_polite() {
        assert!(looks_like_intermediate_ack(
            "好的，我来帮你查看一下这个问题",
            &empty(),
        ));
    }

    #[test]
    fn chinese_plan_to() {
        assert!(looks_like_intermediate_ack(
            "我打算先检查一下配置文件",
            &empty()
        ));
    }

    // ── Category 2: Permission seeking ──

    #[test]
    fn would_you_like() {
        assert!(looks_like_intermediate_ack(
            "Would you like me to check the error logs?",
            &empty(),
        ));
    }

    #[test]
    fn shall_i() {
        assert!(looks_like_intermediate_ack(
            "Shall I investigate the test failure?",
            &empty(),
        ));
    }

    #[test]
    fn should_i() {
        assert!(looks_like_intermediate_ack(
            "Should I run the tests to verify?",
            &empty(),
        ));
    }

    #[test]
    fn chinese_permission() {
        assert!(looks_like_intermediate_ack(
            "要不要我帮你检查一下这个问题？",
            &empty(),
        ));
    }

    // ── Category 3: Self-narration ──

    #[test]
    fn heres_my_plan() {
        assert!(looks_like_intermediate_ack(
            "Here's my plan: first I'll read the config, then run the tests.",
            &empty(),
        ));
    }

    #[test]
    fn my_approach_is() {
        assert!(looks_like_intermediate_ack(
            "My approach is to investigate the failing module and fix the imports.",
            &empty(),
        ));
    }

    #[test]
    fn chinese_approach() {
        assert!(looks_like_intermediate_ack(
            "我的思路是先排查配置文件的问题",
            &empty(),
        ));
    }

    #[test]
    fn chinese_steps() {
        assert!(looks_like_intermediate_ack(
            "具体步骤如下：第一步检查日志，第二步修复配置",
            &empty(),
        ));
    }

    // ── Mid-turn detection (tools called earlier, model still lazy) ──

    #[test]
    fn detects_after_tools_when_last_msg_is_user() {
        assert!(looks_like_intermediate_ack(
            "I'll look into the build failure and check the logs.",
            &after_user(),
        ));
    }

    // ── Negative: should NOT trigger ──

    #[test]
    fn ignores_when_last_msg_is_tool_result() {
        assert!(!looks_like_intermediate_ack(
            "I'll look into the build failure.",
            &after_tool(),
        ));
    }

    #[test]
    fn ignores_long_response_no_patterns() {
        // Long response with no lazy patterns at all → pass through.
        let long = "The configuration looks standard. ".repeat(100);
        assert!(!looks_like_intermediate_ack(&long, &empty()));
    }

    #[test]
    fn ignores_genuine_answer() {
        assert!(!looks_like_intermediate_ack(
            "The build succeeded. All 42 tests passed.",
            &empty(),
        ));
    }

    #[test]
    fn ignores_empty() {
        assert!(!looks_like_intermediate_ack("", &empty()));
    }

    #[test]
    fn word_boundary_no_false_positive() {
        assert!(!looks_like_intermediate_ack(
            "I filled the test report and checked the results.",
            &empty(),
        ));
    }

    #[test]
    fn ignores_ack_inside_think_block() {
        assert!(!looks_like_intermediate_ack(
            "<think>I'll look into this and check the logs.</think>The answer is 42.",
            &empty(),
        ));
    }

    #[test]
    fn think_strip_preserves_visible_ack() {
        assert!(looks_like_intermediate_ack(
            "<think>reasoning</think>Let me check the build logs.",
            &empty(),
        ));
    }

    // ── Result phrase exclusion ──

    #[test]
    fn ignores_response_with_result() {
        assert!(!looks_like_intermediate_ack(
            "I'll summarize what I found. Here is the configuration issue.",
            &empty(),
        ));
    }

    #[test]
    fn ignores_chinese_result() {
        assert!(!looks_like_intermediate_ack(
            "让我总结一下，问题是配置文件缺少必要字段",
            &empty(),
        ));
    }

    #[test]
    fn ignores_completed_action() {
        assert!(!looks_like_intermediate_ack(
            "I've fixed the import error and updated the config.",
            &empty(),
        ));
    }

    #[test]
    fn ignores_chinese_completed() {
        assert!(!looks_like_intermediate_ack(
            "已经完成了配置文件的修复，搞定了",
            &empty(),
        ));
    }

    // ── Tier 2: Trailing intent (verbose narration with lazy tail) ──

    #[test]
    fn catches_verbose_chinese_trailing_intent() {
        // GPT's classic: 3000+ chars of architecture discussion ending with
        // "如果你要，我下一步可以...检查..."
        // Body uses neutral text (no lazy patterns) so only the tail triggers.
        let body = "这个模块的职责划分是合理的，结构清晰，边界明确。".repeat(200);
        let tail = "如果你要，我下一步可以直接替你做最后一轮检查";
        let text = format!("{body}{tail}");
        assert!(text.chars().count() > MAX_ACK_LENGTH_CHARS);
        assert_eq!(detect(&text, &empty()), Some(AckKind::TrailingIntent));
    }

    #[test]
    fn catches_verbose_english_trailing_intent() {
        let body = "The module architecture looks correct. ".repeat(80);
        let tail = " Would you like me to check the configuration and fix the issue?";
        let text = format!("{body}{tail}");
        assert!(text.chars().count() > MAX_ACK_LENGTH_CHARS);
        assert_eq!(detect(&text, &empty()), Some(AckKind::TrailingIntent));
    }

    #[test]
    fn verbose_ignores_genuine_answer() {
        let body = "The module architecture looks correct. ".repeat(80);
        let tail = " I've completed the analysis and the root cause is in the config.";
        let text = format!("{body}{tail}");
        assert!(!looks_like_intermediate_ack(&text, &empty()));
    }

    #[test]
    fn verbose_ignores_result_in_body() {
        let body = format!(
            "{}. Here is what I found: the config is broken. ",
            "x".repeat(2000),
        );
        let tail = "如果你要，我下一步可以检查一下";
        let text = format!("{body}{tail}");
        assert!(!looks_like_intermediate_ack(&text, &empty()));
    }

    #[test]
    fn verbose_ignores_very_long_response() {
        let text = format!("{}如果你要，我来检查一下", "x".repeat(9000));
        assert!(!looks_like_intermediate_ack(&text, &empty()));
    }

    #[test]
    fn verbose_ignores_clean_tail() {
        // Long response with neutral body and no lazy pattern in the tail.
        let body = "The configuration is standard. ".repeat(100);
        let tail = " Everything follows best practices and no changes are needed.";
        let text = format!("{body}{tail}");
        assert!(text.chars().count() > MAX_ACK_LENGTH_CHARS);
        assert!(!looks_like_intermediate_ack(&text, &empty()));
    }

    // ── Tier 3: Dense narration (5+ intent phrases) ──

    #[test]
    fn catches_dense_english_narration() {
        // Planning essay with 6 intent phrases, padded to exceed 2000 chars.
        let text = format!(
            "{}First, I'll check the logs. Then I need to review the config. Let me also inspect \
             the database. I should verify the migrations. I'm going to investigate the API \
             layer. Allow me to examine the tests.",
            "x".repeat(2200),
        );
        assert!(text.chars().count() > MAX_ACK_LENGTH_CHARS);
        assert_eq!(detect(&text, &empty()), Some(AckKind::DenseNarration));
    }

    #[test]
    fn catches_dense_chinese_narration() {
        // Planning essay with 6+ Chinese intent phrases
        let text = format!(
            "{}我先检查一下日志。然后我来看看配置文件。接下来我去确认数据库迁移。我打算分析API层。\
             首先我要验证测试用例。后续我处理一下部署问题。",
            "x".repeat(2200),
        );
        assert!(text.chars().count() > MAX_ACK_LENGTH_CHARS);
        assert_eq!(detect(&text, &empty()), Some(AckKind::DenseNarration));
    }

    #[test]
    fn dense_ignores_below_threshold() {
        // Only 2 intent phrases — not dense enough
        let text = format!(
            "{}I'll check the logs. Let me review the config. Everything else looks fine and the \
             tests pass.",
            "x".repeat(2000),
        );
        assert!(text.chars().count() > MAX_ACK_LENGTH_CHARS);
        // Should NOT be DenseNarration (only 2 phrases)
        // Might be TrailingIntent if tail matches
        assert_ne!(detect(&text, &empty()), Some(AckKind::DenseNarration));
    }

    #[test]
    fn dense_ignores_with_result_phrase() {
        let text = format!(
            "{}I'll check X. Let me do Y. I need to fix Z. I should handle W. First, I'll verify \
             V. Here is the result of my analysis.",
            "x".repeat(1800),
        );
        // Result phrase → None
        assert!(!looks_like_intermediate_ack(&text, &empty()));
    }

    // ── AckKind nudge messages ──

    #[test]
    fn short_ack_returns_correct_kind() {
        assert_eq!(
            detect(
                "I'll look into the build failure and check the logs.",
                &empty()
            ),
            Some(AckKind::ShortAck),
        );
    }

    #[test]
    fn nudge_messages_differ_by_kind() {
        assert_ne!(
            AckKind::ShortAck.nudge_message(),
            AckKind::TrailingIntent.nudge_message()
        );
        assert_ne!(
            AckKind::TrailingIntent.nudge_message(),
            AckKind::DenseNarration.nudge_message()
        );
    }
}
