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

//! Runtime helpers for WeChat message processing.
//!
//! Ported from
//! [wechat-agent-rs](https://github.com/rararulab/wechat-agent-rs).
//! Only the functions used by the adapter are included here:
//! [`body_from_item_list`] and [`markdown_to_plain_text`].

use serde_json::Value;

/// Strips Markdown formatting from text, returning a plain-text
/// approximation.
pub fn markdown_to_plain_text(text: &str) -> String {
    let mut result = text.to_string();
    let code_block_re = regex_lite::Regex::new(r"(?s)```[\s\S]*?```").expect("valid regex");
    result = code_block_re.replace_all(&result, "").to_string();
    let inline_code_re = regex_lite::Regex::new(r"`[^`]+`").expect("valid regex");
    result = inline_code_re.replace_all(&result, "").to_string();
    let img_re = regex_lite::Regex::new(r"!\[([^\]]*)\]\([^)]+\)").expect("valid regex");
    result = img_re.replace_all(&result, "$1").to_string();
    let link_re = regex_lite::Regex::new(r"\[([^\]]+)\]\([^)]+\)").expect("valid regex");
    result = link_re.replace_all(&result, "$1").to_string();
    let bold_re = regex_lite::Regex::new(r"\*\*([^*]+)\*\*").expect("valid regex");
    result = bold_re.replace_all(&result, "$1").to_string();
    let italic_re = regex_lite::Regex::new(r"\*([^*]+)\*").expect("valid regex");
    result = italic_re.replace_all(&result, "$1").to_string();
    let strike_re = regex_lite::Regex::new(r"~~([^~]+)~~").expect("valid regex");
    result = strike_re.replace_all(&result, "$1").to_string();
    let table_sep_re = regex_lite::Regex::new(r"\|[-:| ]+\|").expect("valid regex");
    result = table_sep_re.replace_all(&result, "").to_string();
    result.replace('|', " ").trim().to_string()
}

/// Extracts the text body from a WeChat `item_list` JSON array.
pub fn body_from_item_list(item_list: &[Value]) -> String {
    let mut parts = vec![];
    for item in item_list {
        let item_type = item["type"].as_u64().unwrap_or(0);
        match item_type {
            // Legacy format: type 0 with top-level "body" field.
            0 => {
                if let Some(body) = item["body"].as_str() {
                    parts.push(body.to_string());
                }
            }
            // Current iLink API format: type 1 with nested "text_item.text".
            1 => {
                if let Some(text) = item["text_item"]["text"].as_str() {
                    parts.push(text.to_string());
                }
            }
            5 => {
                if let Some(trans) = item["voice_transcription_body"].as_str() {
                    parts.push(trans.to_string());
                }
            }
            7 => {
                if let Some(ref_list) = item["ref_item_list"].as_array() {
                    let ref_text = body_from_item_list(ref_list);
                    if !ref_text.is_empty() {
                        parts.push(format!("> {ref_text}"));
                    }
                }
            }
            _ => {}
        }
    }
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- markdown_to_plain_text tests --

    #[test]
    fn test_strip_code_blocks() {
        let input = "before\n```rust\nfn main() {}\n```\nafter";
        let result = markdown_to_plain_text(input);
        assert_eq!(result, "before\n\nafter");
    }

    #[test]
    fn test_strip_inline_code() {
        let input = "use `println!` macro";
        let result = markdown_to_plain_text(input);
        assert_eq!(result, "use  macro");
    }

    #[test]
    fn test_strip_bold() {
        let input = "this is **bold** text";
        let result = markdown_to_plain_text(input);
        assert_eq!(result, "this is bold text");
    }

    #[test]
    fn test_strip_italic() {
        let input = "this is *italic* text";
        let result = markdown_to_plain_text(input);
        assert_eq!(result, "this is italic text");
    }

    #[test]
    fn test_strip_strikethrough() {
        let input = "this is ~~deleted~~ text";
        let result = markdown_to_plain_text(input);
        assert_eq!(result, "this is deleted text");
    }

    #[test]
    fn test_strip_links() {
        let input = "click [here](https://example.com) now";
        let result = markdown_to_plain_text(input);
        assert_eq!(result, "click here now");
    }

    #[test]
    fn test_strip_images() {
        let input = "see ![my image](https://example.com/img.png) above";
        let result = markdown_to_plain_text(input);
        assert_eq!(result, "see my image above");
    }

    #[test]
    fn test_strip_tables() {
        let input = "| Name | Age |\n|------|-----|\n| Alice | 30 |";
        let result = markdown_to_plain_text(input);
        assert!(
            !result.contains('|'),
            "pipes should be removed, got: {result}"
        );
        assert!(
            !result.contains("---"),
            "table separator should be removed, got: {result}"
        );
    }

    #[test]
    fn test_plain_text_passthrough() {
        let input = "Hello, this is plain text.";
        let result = markdown_to_plain_text(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_mixed_markdown() {
        let input = "**bold** and *italic* and [link](http://x.com)";
        let result = markdown_to_plain_text(input);
        assert_eq!(result, "bold and italic and link");
    }

    // -- body_from_item_list tests --

    #[test]
    fn test_text_item_legacy() {
        let items = vec![serde_json::json!({"type": 0, "body": "hello world"})];
        let result = body_from_item_list(&items);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_text_item_ilink_v2() {
        let items = vec![serde_json::json!({"type": 1, "text_item": {"text": "深深的"}})];
        let result = body_from_item_list(&items);
        assert_eq!(result, "深深的");
    }

    #[test]
    fn test_voice_transcription() {
        let items = vec![serde_json::json!({
            "type": 5,
            "voice_transcription_body": "transcribed text"
        })];
        let result = body_from_item_list(&items);
        assert_eq!(result, "transcribed text");
    }

    #[test]
    fn test_quoted_message() {
        let items = vec![serde_json::json!({
            "type": 7,
            "ref_item_list": [{"type": 0, "body": "original message"}]
        })];
        let result = body_from_item_list(&items);
        assert_eq!(result, "> original message");
    }

    #[test]
    fn test_multiple_items() {
        let items = vec![
            serde_json::json!({"type": 0, "body": "first"}),
            serde_json::json!({"type": 0, "body": "second"}),
        ];
        let result = body_from_item_list(&items);
        assert_eq!(result, "first\nsecond");
    }

    #[test]
    fn test_empty_list() {
        let items: Vec<Value> = vec![];
        let result = body_from_item_list(&items);
        assert_eq!(result, "");
    }

    #[test]
    fn test_unknown_type() {
        let items = vec![serde_json::json!({"type": 99, "body": "ignored"})];
        let result = body_from_item_list(&items);
        assert_eq!(result, "");
    }
}
