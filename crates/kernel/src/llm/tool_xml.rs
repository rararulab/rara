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

//! Incremental parser for XML-format tool calls in LLM content streams.
//!
//! Some providers (MiniMax) emit tool invocations as XML in the content
//! stream rather than as structured `tool_calls` in the SSE chunk:
//!
//! ```xml
//! <invoke name="ctx_fetch_and_index">
//! <parameter name="url">https://example.com</parameter>
//! </invoke>
//! ```
//!
//! This parser intercepts those blocks and converts them into structured
//! tool call data, following the same incremental streaming pattern as
//! [`super::think_tag::ThinkTagParser`].

/// A classified segment from the content stream.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Segment {
    /// User-visible text that is not part of a tool call.
    Text(String),
    /// A fully parsed XML tool invocation.
    ToolCall {
        name:      String,
        arguments: serde_json::Value,
    },
}

const OPEN_TAG: &str = "<invoke ";
const CLOSE_TAG: &str = "</invoke>";
/// Trailing tag some models emit after `</invoke>`. Stripped silently.
const MINIMAX_CLOSE: &str = "</minimax:tool_call>";

/// Incremental parser that separates XML tool calls from regular text.
///
/// Buffers content between `<invoke ...>` and `</invoke>`, parses the
/// tool name and parameters, and emits [`Segment::ToolCall`].  Everything
/// outside those blocks passes through as [`Segment::Text`].
pub(crate) struct ToolXmlParser {
    /// Whether we are inside an `<invoke>...</invoke>` block.
    inside:  bool,
    /// Accumulated text not yet emitted (may contain partial tags).
    pending: String,
}

impl ToolXmlParser {
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
        self.drain(&mut out);
        out
    }

    /// Flush remaining buffered content when the stream ends.
    pub(crate) fn flush(&mut self) -> Vec<Segment> {
        if self.pending.is_empty() {
            return Vec::new();
        }
        let text = std::mem::take(&mut self.pending);
        if self.inside {
            // Incomplete XML block — emit as text so nothing is silently lost.
            vec![Segment::Text(text)]
        } else {
            vec![Segment::Text(text)]
        }
    }

    fn drain(&mut self, out: &mut Vec<Segment>) {
        loop {
            if self.inside {
                // Look for closing tag.
                let Some(pos) = self.pending.find(CLOSE_TAG) else {
                    // Not enough data yet — wait for more chunks.
                    break;
                };
                let xml_body = self.pending[..pos].to_owned();
                let after = self.pending[pos + CLOSE_TAG.len()..].to_owned();
                self.pending = after;
                self.inside = false;

                // Strip trailing </minimax:tool_call> if present.
                let trimmed = self.pending.trim_start();
                if trimmed.starts_with(MINIMAX_CLOSE) {
                    let skip = self.pending.len() - trimmed.len() + MINIMAX_CLOSE.len();
                    self.pending = self.pending[skip..].to_owned();
                }

                if let Some(tc) = parse_invoke_block(&xml_body) {
                    out.push(tc);
                }
                continue;
            }

            // Outside an invoke block — look for opening tag.
            if let Some(pos) = self.pending.find(OPEN_TAG) {
                let before = self.pending[..pos].to_owned();
                let after = self.pending[pos + OPEN_TAG.len()..].to_owned();

                if !before.is_empty() {
                    out.push(Segment::Text(before));
                }

                // `after` starts right after `<invoke ` — the rest of
                // the opening tag (name="..."> etc) plus body.
                self.pending = after;
                self.inside = true;
                continue;
            }

            // No full open tag found.  Emit text that is definitely
            // safe (cannot be a partial `<invoke ` prefix).
            let safe = safe_emit_len(&self.pending, OPEN_TAG);
            if safe == 0 {
                break;
            }
            let emit = self.pending[..safe].to_owned();
            self.pending = self.pending[safe..].to_owned();
            if !emit.is_empty() {
                out.push(Segment::Text(emit));
            }
            break;
        }
    }
}

/// How many bytes at the start of `text` can be safely emitted without
/// splitting a potential partial `tag` at the tail.
///
/// Only iterates valid UTF-8 char boundaries via `char_indices()` to
/// avoid panicking on multi-byte characters (e.g. CJK punctuation).
fn safe_emit_len(text: &str, tag: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    // Walk char boundaries from the end. If the tail starting at any
    // boundary is a prefix of `tag`, we must keep it buffered.
    let boundaries: Vec<usize> = text
        .char_indices()
        .map(|(idx, _)| idx)
        .chain(std::iter::once(text.len()))
        .collect();
    for &start_idx in boundaries.iter().rev() {
        let tail = &text[start_idx..];
        if !tail.is_empty() && tag.starts_with(tail) {
            return start_idx;
        }
    }
    text.len()
}

/// Parse the body between `<invoke ` and `</invoke>` into a
/// [`Segment::ToolCall`].
///
/// Expected format (after stripping `<invoke `):
/// ```text
/// name="tool_name">
/// <parameter name="key1">value1</parameter>
/// <parameter name="key2">value2</parameter>
/// ```
fn parse_invoke_block(body: &str) -> Option<Segment> {
    // Extract tool name from `name="...">`
    let name = extract_attr(body, "name")?;

    // Extract all <parameter name="key">value</parameter> pairs.
    let mut args = serde_json::Map::new();
    let mut search_from = 0;
    while let Some(start) = body[search_from..].find("<parameter ") {
        let abs_start = search_from + start;
        let param_body = &body[abs_start..];

        if let Some(end) = param_body.find("</parameter>") {
            let inner = &param_body[..end];
            if let Some(key) = extract_attr(inner, "name") {
                // Value is between `>` (end of opening tag) and the start
                // of `</parameter>`.
                if let Some(gt) = inner.find('>') {
                    let value = inner[gt + 1..].trim();
                    args.insert(key, serde_json::Value::String(value.to_owned()));
                }
            }
            search_from = abs_start + end + "</parameter>".len();
        } else {
            break;
        }
    }

    Some(Segment::ToolCall {
        name,
        arguments: serde_json::Value::Object(args),
    })
}

/// Extract the value of an XML attribute: `name="value"` → `"value"`.
fn extract_attr(text: &str, attr: &str) -> Option<String> {
    let pattern = format!("{attr}=\"");
    let start = text.find(&pattern)? + pattern.len();
    let end = text[start..].find('"')? + start;
    Some(text[start..end].to_owned())
}

/// Strip XML tool calls from a complete string (non-streaming path).
///
/// Returns `(tool_calls, cleaned_text)`.
pub(crate) fn strip_tool_xml(text: &str) -> (Vec<Segment>, String) {
    let mut parser = ToolXmlParser::new();
    let mut tools = Vec::new();
    let mut visible = String::new();
    for segment in parser.push(text).into_iter().chain(parser.flush()) {
        match segment {
            Segment::Text(t) => visible.push_str(&t),
            tc @ Segment::ToolCall { .. } => tools.push(tc),
        }
    }
    (tools, visible)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect_stream(chunks: &[&str]) -> (String, Vec<(String, serde_json::Value)>) {
        let mut parser = ToolXmlParser::new();
        let mut text = String::new();
        let mut calls = Vec::new();

        for chunk in chunks {
            for seg in parser.push(chunk) {
                match seg {
                    Segment::Text(t) => text.push_str(&t),
                    Segment::ToolCall { name, arguments } => calls.push((name, arguments)),
                }
            }
        }
        for seg in parser.flush() {
            match seg {
                Segment::Text(t) => text.push_str(&t),
                Segment::ToolCall { name, arguments } => calls.push((name, arguments)),
            }
        }

        (text, calls)
    }

    #[test]
    fn no_xml_tags() {
        let (text, calls) = collect_stream(&["Hello world"]);
        assert_eq!(text, "Hello world");
        assert!(calls.is_empty());
    }

    #[test]
    fn complete_invoke_block() {
        let (text, calls) = collect_stream(&[
            r#"before<invoke name="fetch"><parameter name="url">https://example.com</parameter></invoke>after"#,
        ]);
        assert_eq!(text, "beforeafter");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "fetch");
        assert_eq!(calls[0].1["url"], "https://example.com");
    }

    #[test]
    fn streaming_partial_open_tag() {
        let (text, calls) = collect_stream(&[
            "text before<inv",
            r#"oke name="tool"><parameter name="key">val</parameter></invoke>after"#,
        ]);
        assert_eq!(text, "text beforeafter");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "tool");
    }

    #[test]
    fn streaming_partial_close_tag() {
        let (text, calls) = collect_stream(&[
            r#"<invoke name="x"><parameter name="a">1</parameter></invo"#,
            "ke>done",
        ]);
        assert_eq!(text, "done");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "x");
    }

    #[test]
    fn minimax_trailing_tag_stripped() {
        let (text, calls) = collect_stream(&[
            r#"<invoke name="fetch"><parameter name="url">https://example.com</parameter></invoke>
</minimax:tool_call>rest"#,
        ]);
        assert_eq!(text.trim(), "rest");
        assert_eq!(calls.len(), 1);
    }

    #[test]
    fn multiple_invoke_blocks() {
        let (text, calls) = collect_stream(&[
            r#"<invoke name="a"><parameter name="x">1</parameter></invoke>mid<invoke name="b"><parameter name="y">2</parameter></invoke>"#,
        ]);
        assert_eq!(text, "mid");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "a");
        assert_eq!(calls[1].0, "b");
    }

    #[test]
    fn strip_from_complete_string() {
        let input = r#"before<invoke name="t"><parameter name="k">v</parameter></invoke>after"#;
        let (tools, visible) = strip_tool_xml(input);
        assert_eq!(visible, "beforeafter");
        assert_eq!(tools.len(), 1);
    }

    #[test]
    fn false_alarm_angle_bracket() {
        let (text, calls) = collect_stream(&["x < y and <b>bold</b>"]);
        assert_eq!(text, "x < y and <b>bold</b>");
        assert!(calls.is_empty());
    }

    #[test]
    fn cjk_text_no_panic() {
        // Regression: safe_emit_len used raw byte indices which panicked
        // on multi-byte UTF-8 characters (。is 3 bytes: 862..865).
        let cjk = "在 Memoh 里，Bot 是一个完整的 AI Agent 实例。具备专属记忆。";
        let (text, calls) = collect_stream(&[cjk]);
        assert_eq!(text, cjk);
        assert!(calls.is_empty());
    }

    #[test]
    fn cjk_with_partial_open_tag() {
        // CJK text ending with a partial `<invoke ` prefix.
        let (text, calls) = collect_stream(&[
            "你好<inv",
            r#"oke name="t"><parameter name="k">v</parameter></invoke>"#,
        ]);
        assert_eq!(text, "你好");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "t");
    }

    #[test]
    fn multiple_parameters() {
        let (_, calls) = collect_stream(&[
            r#"<invoke name="fetch"><parameter name="source">My Source</parameter><parameter name="url">https://example.com</parameter></invoke>"#,
        ]);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1["source"], "My Source");
        assert_eq!(calls[0].1["url"], "https://example.com");
    }
}
