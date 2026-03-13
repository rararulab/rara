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

//! Lightweight mood inference from conversation messages.
//!
//! Uses keyword/heuristic analysis on the last few assistant replies to infer
//! the emotional tone of the conversation.  No LLM calls are made — this is
//! designed to be cheap enough to run at the end of every agent loop turn.

use rara_soul::MoodLabel;
use tracing::warn;

use crate::llm;

/// How many trailing assistant messages to inspect.
const TAIL_WINDOW: usize = 5;

// ── Keyword sets ────────────────────────────────────────────────────────────

const CHEERFUL_KEYWORDS: &[&str] = &[
    "哈哈",
    "haha",
    "😄",
    "😊",
    "太好了",
    "great",
    "awesome",
    "wonderful",
    "棒",
    "nice",
    "excited",
    "开心",
    "高兴",
    "耶",
    "🎉",
    "excellent",
];

const PLAYFUL_KEYWORDS: &[&str] = &[
    "哈", "lol", "😂", "🤣", "有趣", "funny", "joke", "玩笑", "好玩", "逗", "嘻嘻", "quirky",
    "silly",
];

const FOCUSED_KEYWORDS: &[&str] = &[
    "```",
    "fn ",
    "struct ",
    "impl ",
    "error[",
    "warning[",
    "SELECT ",
    "CREATE ",
    "ALTER ",
    "INSERT ",
    "def ",
    "class ",
    "import ",
    "from ",
    "module",
    "migration",
    "schema",
    "query",
    "debug",
    "trace",
];

const APOLOGY_KEYWORDS: &[&str] = &[
    "抱歉",
    "sorry",
    "my mistake",
    "我的错",
    "修正",
    "correction",
    "oops",
    "不好意思",
    "apologize",
];

/// Result of mood inference: a label and a confidence score.
#[derive(Debug, Clone, PartialEq)]
pub struct MoodInference {
    pub label:      MoodLabel,
    pub confidence: f32,
}

/// Analyse the tail of the conversation and infer the current mood.
///
/// Returns `None` when no clear signal is found (i.e. keep the existing mood).
pub fn infer_mood(messages: &[llm::Message]) -> Option<MoodInference> {
    // Collect text from the last few assistant messages.
    let assistant_texts: Vec<&str> = messages
        .iter()
        .rev()
        .filter(|m| m.role == llm::Role::Assistant)
        .take(TAIL_WINDOW)
        .map(|m| m.content.as_text())
        .collect();

    if assistant_texts.is_empty() {
        return None;
    }

    let combined: String = assistant_texts.join("\n");
    let lower = combined.to_lowercase();

    // Count hits in each category.
    let cheerful_hits = count_hits(&lower, &combined, CHEERFUL_KEYWORDS);
    let playful_hits = count_hits(&lower, &combined, PLAYFUL_KEYWORDS);
    let focused_hits = count_hits(&lower, &combined, FOCUSED_KEYWORDS);
    let apology_hits = count_hits(&lower, &combined, APOLOGY_KEYWORDS);

    // Apology → keep current mood (return None).
    if apology_hits > 0 && cheerful_hits == 0 && playful_hits == 0 {
        return None;
    }

    // Pick the category with the most hits.
    let max_hits = cheerful_hits.max(playful_hits).max(focused_hits);
    if max_hits == 0 {
        // No signals detected → default to Calm with low confidence.
        return Some(MoodInference {
            label:      MoodLabel::Calm,
            confidence: 0.3,
        });
    }

    // Confidence scales with hit count: 1 hit → 0.4, 2 → 0.55, 3 → 0.7, 4+ → 0.85,
    // cap 0.9.
    let confidence = (max_hits as f32).mul_add(0.15, 0.25).min(0.9);

    let label = if cheerful_hits >= playful_hits && cheerful_hits >= focused_hits {
        MoodLabel::Cheerful
    } else if playful_hits >= focused_hits {
        MoodLabel::Playful
    } else {
        MoodLabel::Focused
    };

    Some(MoodInference { label, confidence })
}

/// Count how many keywords from the set appear in the text.
///
/// Case-sensitive keywords (e.g. `fn `, `struct `) are matched against the
/// original text; everything else is matched against the lowercased version.
fn count_hits(lower: &str, original: &str, keywords: &[&str]) -> usize {
    keywords
        .iter()
        .filter(|kw| {
            // Keywords that contain uppercase or start with a backtick/code
            // marker should be matched case-sensitively against the original.
            let is_case_sensitive = kw.chars().any(|c| c.is_uppercase()) || kw.starts_with('`');
            if is_case_sensitive {
                original.contains(*kw)
            } else {
                lower.contains(*kw)
            }
        })
        .count()
}

/// Best-effort mood update for a soul-bearing agent.
///
/// Loads the soul state, applies the inferred mood, and saves back.
/// Any error is logged at warn level and silently swallowed — mood persistence
/// must never block the agent response path.
pub fn update_soul_mood(agent_name: &str, inference: &MoodInference) {
    let result = (|| -> Result<(), rara_soul::SoulError> {
        let mut state = rara_soul::loader::load_state(agent_name)?.unwrap_or_default();
        state.update_mood(inference.label, inference.confidence);
        rara_soul::loader::save_state(agent_name, &state)?;
        Ok(())
    })();

    if let Err(e) = result {
        warn!(
            agent = agent_name,
            error = %e,
            "failed to persist mood update — continuing without it"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::Message;

    fn assistant_msg(text: &str) -> Message { Message::assistant(text) }

    fn user_msg(text: &str) -> Message { Message::user(text) }

    #[test]
    fn empty_messages_returns_none() {
        assert!(infer_mood(&[]).is_none());
    }

    #[test]
    fn no_assistant_messages_returns_none() {
        let msgs = vec![user_msg("hello"), user_msg("world")];
        assert!(infer_mood(&msgs).is_none());
    }

    #[test]
    fn cheerful_keywords_detected() {
        let msgs = vec![
            user_msg("how are you?"),
            assistant_msg("太好了！这个功能终于完成了，haha awesome!"),
        ];
        let result = infer_mood(&msgs).unwrap();
        assert_eq!(result.label, MoodLabel::Cheerful);
        assert!(result.confidence >= 0.4);
    }

    #[test]
    fn playful_keywords_detected() {
        let msgs = vec![
            user_msg("tell me a joke"),
            assistant_msg("哈，这个笑话好有趣 lol 😂"),
        ];
        let result = infer_mood(&msgs).unwrap();
        assert_eq!(result.label, MoodLabel::Playful);
    }

    #[test]
    fn focused_on_code() {
        let msgs = vec![
            user_msg("fix the bug"),
            assistant_msg(
                "我来看看这个 error:\n```rust\nfn main() {\n    let x = struct Foo;\n}\n```",
            ),
        ];
        let result = infer_mood(&msgs).unwrap();
        assert_eq!(result.label, MoodLabel::Focused);
    }

    #[test]
    fn apology_keeps_current_mood() {
        let msgs = vec![
            user_msg("that's wrong"),
            assistant_msg("抱歉，我的错，让我修正一下"),
        ];
        assert!(infer_mood(&msgs).is_none());
    }

    #[test]
    fn no_signals_defaults_to_calm() {
        let msgs = vec![
            user_msg("what time is it?"),
            assistant_msg("现在是下午三点。"),
        ];
        let result = infer_mood(&msgs).unwrap();
        assert_eq!(result.label, MoodLabel::Calm);
        assert!((result.confidence - 0.3).abs() < f32::EPSILON);
    }

    #[test]
    fn confidence_scales_with_hits() {
        // 1 hit
        let msgs1 = vec![assistant_msg("太好了")];
        let r1 = infer_mood(&msgs1).unwrap();

        // 3 hits
        let msgs3 = vec![assistant_msg("太好了！awesome! haha 真棒")];
        let r3 = infer_mood(&msgs3).unwrap();

        assert!(r3.confidence > r1.confidence);
    }

    #[test]
    fn only_inspects_tail_window() {
        // Put cheerful messages far back, recent ones are neutral.
        let mut msgs: Vec<Message> = Vec::new();
        for _ in 0..10 {
            msgs.push(assistant_msg("太好了 haha awesome!"));
        }
        // Recent assistant messages are neutral.
        for _ in 0..TAIL_WINDOW {
            msgs.push(user_msg("ok"));
            msgs.push(assistant_msg("好的。"));
        }
        let result = infer_mood(&msgs).unwrap();
        assert_eq!(result.label, MoodLabel::Calm);
    }

    #[test]
    fn mixed_signals_picks_strongest() {
        let msgs = vec![assistant_msg(
            "太好了！awesome! great! 来看看代码 ```rust\nfn foo() {}\n```",
        )];
        let result = infer_mood(&msgs).unwrap();
        // 3 cheerful hits vs 2 focused hits → Cheerful wins.
        assert_eq!(result.label, MoodLabel::Cheerful);
    }
}
