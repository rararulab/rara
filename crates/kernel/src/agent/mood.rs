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

//! Pure, sync mood inference over the assistant text accumulated during a
//! turn.
//!
//! Mirrors the keyword-heuristic implementation in `crate::mood` but carries
//! no dependency on `rara_soul` — the `MoodLabel` is surfaced as a
//! snake-case `&'static str` (matching its `serde` representation) so the
//! sans-IO state machine can compute and carry the inference without
//! pulling soul-state IO into the pure layer.
//!
//! The runner re-hydrates the label into `rara_soul::MoodLabel` at the
//! boundary when it persists the mood.

use crate::agent::effect::MoodInference;

/// How many trailing assistant messages to inspect when inferring mood.
const TAIL_WINDOW: usize = 5;

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

/// Infer the mood over the tail of assistant-text samples collected during a
/// turn. Input is newest-first or oldest-first; only `TAIL_WINDOW` entries
/// closest to the end are consulted.
///
/// Returns `None` when there is no clear signal — apology-only or empty
/// input — matching the legacy "keep current mood" semantics.
pub fn infer_mood(assistant_texts: &[String]) -> Option<MoodInference> {
    if assistant_texts.is_empty() {
        return None;
    }
    let start = assistant_texts.len().saturating_sub(TAIL_WINDOW);
    let tail = &assistant_texts[start..];
    let combined: String = tail.join("\n");
    let lower = combined.to_lowercase();

    let cheerful_hits = count_hits(&lower, &combined, CHEERFUL_KEYWORDS);
    let playful_hits = count_hits(&lower, &combined, PLAYFUL_KEYWORDS);
    let focused_hits = count_hits(&lower, &combined, FOCUSED_KEYWORDS);
    let apology_hits = count_hits(&lower, &combined, APOLOGY_KEYWORDS);

    // Apology with no positive signal → keep current mood.
    if apology_hits > 0 && cheerful_hits == 0 && playful_hits == 0 {
        return None;
    }

    let max_hits = cheerful_hits.max(playful_hits).max(focused_hits);
    if max_hits == 0 {
        return Some(MoodInference {
            label:      "calm",
            confidence: 0.3,
        });
    }

    // Confidence scales with hit count: 1 hit → 0.4, 2 → 0.55, 3 → 0.7,
    // 4+ → 0.85, capped at 0.9. Matches legacy `crate::mood` exactly.
    let confidence = (max_hits as f32).mul_add(0.15, 0.25).min(0.9);

    let label = if cheerful_hits >= playful_hits && cheerful_hits >= focused_hits {
        "cheerful"
    } else if playful_hits >= focused_hits {
        "playful"
    } else {
        "focused"
    };

    Some(MoodInference { label, confidence })
}

/// Case-sensitive keywords (uppercase letters or a leading backtick) are
/// matched against the original text; everything else is lowercased first.
fn count_hits(lower: &str, original: &str, keywords: &[&str]) -> usize {
    keywords
        .iter()
        .filter(|kw| {
            let case_sensitive = kw.chars().any(char::is_uppercase) || kw.starts_with('`');
            if case_sensitive {
                original.contains(*kw)
            } else {
                lower.contains(*kw)
            }
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_returns_none() {
        assert!(infer_mood(&[]).is_none());
    }

    #[test]
    fn apology_only_returns_none() {
        let msgs = vec!["抱歉，我的错，让我修正一下".to_owned()];
        assert!(infer_mood(&msgs).is_none());
    }

    #[test]
    fn cheerful_keywords_detected() {
        let msgs = vec!["太好了！这个功能终于完成了，haha awesome!".to_owned()];
        let inf = infer_mood(&msgs).expect("should infer");
        assert_eq!(inf.label, "cheerful");
        assert!(inf.confidence >= 0.4);
    }

    #[test]
    fn focused_on_code() {
        let msgs = vec![
            "我来看看这个 error:\n```rust\nfn main() {\n    let x = struct Foo;\n}\n```".to_owned(),
        ];
        let inf = infer_mood(&msgs).expect("should infer");
        assert_eq!(inf.label, "focused");
    }

    #[test]
    fn no_signals_defaults_to_calm() {
        let msgs = vec!["现在是下午三点。".to_owned()];
        let inf = infer_mood(&msgs).expect("should infer");
        assert_eq!(inf.label, "calm");
        assert!((inf.confidence - 0.3).abs() < f32::EPSILON);
    }

    #[test]
    fn only_inspects_tail_window() {
        // Older messages are cheerful; recent TAIL_WINDOW messages are neutral.
        let mut msgs: Vec<String> = Vec::new();
        for _ in 0..10 {
            msgs.push("太好了 haha awesome!".to_owned());
        }
        for _ in 0..TAIL_WINDOW {
            msgs.push("好的。".to_owned());
        }
        let inf = infer_mood(&msgs).expect("should infer");
        assert_eq!(inf.label, "calm");
    }

    #[test]
    fn confidence_scales_with_hits() {
        let r1 = infer_mood(&["太好了".to_owned()]).unwrap();
        let r3 = infer_mood(&["太好了！awesome! haha 真棒".to_owned()]).unwrap();
        assert!(r3.confidence > r1.confidence);
    }
}
