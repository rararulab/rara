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

//! Parser for `<think>...</think>` tags embedded in LLM content streams.
//!
//! Some OpenAI-compatible providers place reasoning in `content` wrapped by
//! XML-like tags instead of using `reasoning_content`. This parser separates
//! those segments so callers can route visible text and reasoning
//! independently.

/// A classified segment extracted from content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Segment {
    /// User-visible text outside `<think>...</think>`.
    Text(String),
    /// Reasoning text inside `<think>...</think>`.
    Thinking(String),
}

const OPEN_TAG: &str = "<think>";
const CLOSE_TAG: &str = "</think>";

/// Incremental parser for splitting streamed deltas by think tags.
pub(crate) struct ThinkTagParser {
    inside:  bool,
    pending: String,
}

impl ThinkTagParser {
    pub(crate) fn new() -> Self {
        Self {
            inside:  false,
            pending: String::new(),
        }
    }

    /// Push one streaming fragment and return classified segments.
    pub(crate) fn push(&mut self, text: &str) -> Vec<Segment> {
        self.pending.push_str(text);
        let mut out = Vec::new();
        self.drain_segments(&mut out);
        out
    }

    /// Flush remaining buffered content when stream ends.
    pub(crate) fn flush(&mut self) -> Vec<Segment> {
        if self.pending.is_empty() {
            return Vec::new();
        }
        let text = std::mem::take(&mut self.pending);
        if self.inside {
            vec![Segment::Thinking(text)]
        } else {
            vec![Segment::Text(text)]
        }
    }

    fn drain_segments(&mut self, out: &mut Vec<Segment>) {
        loop {
            let tag = if self.inside { CLOSE_TAG } else { OPEN_TAG };
            if let Some(pos) = self.pending.find(tag) {
                let before = self.pending[..pos].to_owned();
                let after = self.pending[pos + tag.len()..].to_owned();

                if !before.is_empty() {
                    if self.inside {
                        out.push(Segment::Thinking(before));
                    } else {
                        out.push(Segment::Text(before));
                    }
                }

                self.inside = !self.inside;
                self.pending = after;
                continue;
            }

            let safe_len = self.safe_emit_len(tag);
            if safe_len == 0 {
                break;
            }
            let emit = self.pending[..safe_len].to_owned();
            self.pending = self.pending[safe_len..].to_owned();
            if !emit.is_empty() {
                if self.inside {
                    out.push(Segment::Thinking(emit));
                } else {
                    out.push(Segment::Text(emit));
                }
            }
            break;
        }
    }

    /// Return how many bytes are safe to emit without losing potential partial
    /// open/close tags at the tail of `pending`.
    fn safe_emit_len(&self, tag: &str) -> usize {
        if self.pending.is_empty() {
            return 0;
        }

        let keep_tail = tag.len().saturating_sub(1).min(self.pending.len());
        let safe_end = self.pending.len() - keep_tail;

        let starts = self
            .pending
            .char_indices()
            .map(|(idx, _)| idx)
            .chain(std::iter::once(self.pending.len()));
        for start in starts {
            if start < safe_end {
                continue;
            }
            if tag.starts_with(&self.pending[start..]) {
                return start;
            }
        }
        self.pending.len()
    }
}

/// Strip all `<think>...</think>` segments from a complete string.
///
/// Returns `(thinking_content, visible_text)`.
pub(crate) fn strip_think_tags(text: &str) -> (Option<String>, String) {
    let mut parser = ThinkTagParser::new();
    let mut thinking = String::new();
    let mut visible = String::new();
    for segment in parser.push(text).into_iter().chain(parser.flush()) {
        match segment {
            Segment::Text(t) => visible.push_str(&t),
            Segment::Thinking(t) => thinking.push_str(&t),
        }
    }
    (
        if thinking.is_empty() {
            None
        } else {
            Some(thinking)
        },
        visible,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect_stream(chunks: &[&str]) -> (String, String) {
        let mut parser = ThinkTagParser::new();
        let mut text = String::new();
        let mut thinking = String::new();

        for chunk in chunks {
            for seg in parser.push(chunk) {
                match seg {
                    Segment::Text(t) => text.push_str(&t),
                    Segment::Thinking(t) => thinking.push_str(&t),
                }
            }
        }

        for seg in parser.flush() {
            match seg {
                Segment::Text(t) => text.push_str(&t),
                Segment::Thinking(t) => thinking.push_str(&t),
            }
        }

        (text, thinking)
    }

    #[test]
    fn no_think_tags() {
        let (text, thinking) = collect_stream(&["Hello world"]);
        assert_eq!(thinking, "");
        assert_eq!(text, "Hello world");
    }

    #[test]
    fn complete_think_block() {
        let (text, thinking) = collect_stream(&["<think>reasoning here</think>visible text"]);
        assert_eq!(thinking, "reasoning here");
        assert_eq!(text, "visible text");
    }

    #[test]
    fn streaming_partial_open_tag() {
        let (text, thinking) = collect_stream(&["<thi", "nk>inside"]);
        assert_eq!(thinking, "inside");
        assert_eq!(text, "");
    }

    #[test]
    fn streaming_partial_close_tag() {
        let (text, thinking) = collect_stream(&["<think>reason", "ing</thi", "nk>after"]);
        assert_eq!(thinking, "reasoning");
        assert_eq!(text, "after");
    }

    #[test]
    fn false_alarm_partial_tag() {
        let (text, thinking) = collect_stream(&["<thi", "s is not a tag"]);
        assert_eq!(thinking, "");
        assert_eq!(text, "<this is not a tag");
    }

    #[test]
    fn think_at_start_then_text() {
        let (text, thinking) = collect_stream(&["<think>\nLet me think...\n</think>\n\nHello!"]);
        assert_eq!(thinking, "\nLet me think...\n");
        assert_eq!(text, "\n\nHello!");
    }

    #[test]
    fn split_boundaries_around_both_tags() {
        let (text, thinking) = collect_stream(&["before<thi", "nk>mid</th", "ink>after"]);
        assert_eq!(thinking, "mid");
        assert_eq!(text, "beforeafter");
    }

    #[test]
    fn flush_pending_buffer() {
        let mut parser = ThinkTagParser::new();
        let s1 = parser.push("<thi");
        assert_eq!(s1, vec![]);
        let s2 = parser.flush();
        assert_eq!(s2, vec![Segment::Text("<thi".into())]);
    }

    #[test]
    fn strip_from_complete_string() {
        let (thinking, text) = strip_think_tags("<think>reasoning</think>visible");
        assert_eq!(thinking.as_deref(), Some("reasoning"));
        assert_eq!(text, "visible");
    }

    #[test]
    fn strip_no_tags() {
        let (thinking, text) = strip_think_tags("just plain text");
        assert_eq!(thinking, None);
        assert_eq!(text, "just plain text");
    }

    #[test]
    fn multiple_think_blocks() {
        let (thinking, text) = strip_think_tags("a<think>x</think>b<think>y</think>c");
        assert_eq!(thinking.as_deref(), Some("xy"));
        assert_eq!(text, "abc");
    }
}
