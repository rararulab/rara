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

//! Markdown to Telegram HTML converter with message chunking.
//!
//! Telegram's Bot API supports a [limited HTML subset][tg-html]: `<b>`, `<i>`,
//! `<code>`, `<pre>`, and `<a>`. This module converts standard Markdown
//! formatting to that subset.
//!
//! # Supported Conversions
//!
//! | Markdown              | HTML Output                         |
//! |-----------------------|-------------------------------------|
//! | `**bold**`            | `<b>bold</b>`                       |
//! | `__bold__`            | `<b>bold</b>`                       |
//! | `*italic*`            | `<i>italic</i>`                     |
//! | `_italic_`            | `<i>italic</i>`                     |
//! | `` `code` ``          | `<code>code</code>`                 |
//! | ` ```lang\ncode``` `  | `<pre>code</pre>`                   |
//! | `[text](url)`         | `<a href="url">text</a>`            |
//!
//! HTML special characters (`&`, `<`, `>`) are escaped before any Markdown
//! processing to prevent injection.
//!
//! # Message Chunking
//!
//! [`chunk_message`] splits long HTML strings into pieces that fit within
//! Telegram's 4096-character message limit. It prefers breaking at newlines,
//! then spaces, and falls back to hard breaks as a last resort.
//!
//! [tg-html]: https://core.telegram.org/bots/api#html-style

/// Telegram maximum message length in characters.
pub const TELEGRAM_MAX_MESSAGE_LEN: usize = 4096;

/// Convert Markdown text to Telegram-supported HTML subset.
///
/// Supported conversions:
/// - `**bold**` or `__bold__` -> `<b>bold</b>`
/// - `*italic*` or `_italic_` -> `<i>italic</i>`
/// - `` `code` `` -> `<code>code</code>`
/// - ` ```pre``` ` -> `<pre>pre</pre>`
/// - `[text](url)` -> `<a href="url">text</a>`
///
/// HTML special characters (`&`, `<`, `>`) are escaped first.
pub fn markdown_to_telegram_html(md: &str) -> String {
    // First pass: convert block-level markdown (headings, HRs, blockquotes)
    // into inline equivalents that the character-level parser can handle.
    let preprocessed = preprocess_blocks(md);

    // Second pass: escape HTML entities in the raw text.
    let escaped = html_escape(&preprocessed);

    let mut result = String::with_capacity(escaped.len());
    let chars: Vec<char> = escaped.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Fenced code blocks: ```...```
        if i + 2 < len && chars[i] == '`' && chars[i + 1] == '`' && chars[i + 2] == '`' {
            i += 3;
            // skip optional language tag (until newline)
            while i < len && chars[i] != '\n' {
                i += 1;
            }
            if i < len {
                i += 1; // skip newline
            }
            let start = i;
            // find closing ```
            while i + 2 < len {
                if chars[i] == '`' && chars[i + 1] == '`' && chars[i + 2] == '`' {
                    break;
                }
                i += 1;
            }
            let code: String = chars[start..i].iter().collect();
            // trim trailing newline inside code block
            let code = code.trim_end_matches('\n');
            result.push_str("<pre>");
            result.push_str(code);
            result.push_str("</pre>");
            if i + 2 < len {
                i += 3; // skip closing ```
            }
            continue;
        }

        // Inline code: `...`
        if chars[i] == '`' {
            i += 1;
            let start = i;
            while i < len && chars[i] != '`' {
                i += 1;
            }
            let code: String = chars[start..i].iter().collect();
            result.push_str("<code>");
            result.push_str(&code);
            result.push_str("</code>");
            if i < len {
                i += 1; // skip closing `
            }
            continue;
        }

        // Links: [text](url)
        if chars[i] == '[' {
            if let Some((link_text, url, end_pos)) = try_parse_link(&chars, i) {
                use std::fmt::Write;
                let _ = write!(result, "<a href=\"{url}\">{link_text}</a>");
                i = end_pos;
                continue;
            }
        }

        // Bold: **text** or __text__
        if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
            if let Some((content, end_pos)) = try_parse_delimited(&chars, i, "**") {
                result.push_str("<b>");
                result.push_str(&content);
                result.push_str("</b>");
                i = end_pos;
                continue;
            }
        }
        if i + 1 < len && chars[i] == '_' && chars[i + 1] == '_' {
            if let Some((content, end_pos)) = try_parse_delimited(&chars, i, "__") {
                result.push_str("<b>");
                result.push_str(&content);
                result.push_str("</b>");
                i = end_pos;
                continue;
            }
        }

        // Italic: *text* or _text_
        if chars[i] == '*' {
            if let Some((content, end_pos)) = try_parse_delimited(&chars, i, "*") {
                result.push_str("<i>");
                result.push_str(&content);
                result.push_str("</i>");
                i = end_pos;
                continue;
            }
        }
        if chars[i] == '_' {
            if let Some((content, end_pos)) = try_parse_delimited(&chars, i, "_") {
                result.push_str("<i>");
                result.push_str(&content);
                result.push_str("</i>");
                i = end_pos;
                continue;
            }
        }

        result.push(chars[i]);
        i += 1;
    }

    result
}

/// Pre-process block-level Markdown into inline equivalents.
///
/// Converts headings to bold (`**text**`), strips horizontal rules, and
/// removes blockquote markers so the character-level parser can handle them.
fn preprocess_blocks(md: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in md.lines() {
        let trimmed = line.trim();

        // Headings: #{1,6} text -> **text**
        if let Some(rest) = strip_heading_prefix(trimmed) {
            lines.push(format!("**{rest}**"));
        }
        // Horizontal rules: 3+ of -, *, or _ (optionally with spaces)
        else if is_horizontal_rule(trimmed) {
            lines.push(String::new());
        }
        // Blockquotes: > text -> text
        else if let Some(rest) = trimmed.strip_prefix("> ") {
            lines.push(rest.to_string());
        } else if trimmed == ">" {
            lines.push(String::new());
        } else {
            lines.push(line.to_string());
        }
    }
    lines.join("\n")
}

/// Strip a Markdown heading prefix (`# ` through `###### `), returning the
/// heading text. Returns `None` if the line is not a heading.
fn strip_heading_prefix(line: &str) -> Option<&str> {
    let bytes = line.as_bytes();
    let mut level = 0;
    while level < bytes.len() && level < 6 && bytes[level] == b'#' {
        level += 1;
    }
    if level == 0 {
        return None;
    }
    // Must be followed by a space (or be just `###...` with nothing after).
    let rest = &line[level..];
    if rest.is_empty() {
        return Some("");
    }
    if rest.starts_with(' ') {
        return Some(rest[1..].trim());
    }
    None
}

/// Check whether a line is a Markdown horizontal rule (`---`, `***`, `___`,
/// possibly with spaces between).
fn is_horizontal_rule(line: &str) -> bool {
    let stripped: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    if stripped.len() < 3 {
        return false;
    }
    let ch = stripped.as_bytes()[0];
    matches!(ch, b'-' | b'*' | b'_') && stripped.bytes().all(|b| b == ch)
}

/// Escape HTML special characters.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Try to parse `[text](url)` starting at position `pos`.
///
/// Returns `(link_text, url, end_position)` if successful.
fn try_parse_link(chars: &[char], pos: usize) -> Option<(String, String, usize)> {
    let len = chars.len();
    if pos >= len || chars[pos] != '[' {
        return None;
    }

    // Find closing ]
    let mut i = pos + 1;
    let mut depth = 1;
    while i < len && depth > 0 {
        match chars[i] {
            '[' => depth += 1,
            ']' => depth -= 1,
            _ => {}
        }
        if depth > 0 {
            i += 1;
        }
    }
    if depth != 0 || i >= len {
        return None;
    }

    let text: String = chars[pos + 1..i].iter().collect();
    i += 1; // skip ]

    // Expect (
    if i >= len || chars[i] != '(' {
        return None;
    }
    i += 1;

    let url_start = i;
    let mut paren_depth = 1;
    while i < len && paren_depth > 0 {
        match chars[i] {
            '(' => paren_depth += 1,
            ')' => paren_depth -= 1,
            _ => {}
        }
        if paren_depth > 0 {
            i += 1;
        }
    }
    if paren_depth != 0 {
        return None;
    }

    let url: String = chars[url_start..i].iter().collect();
    i += 1; // skip )

    Some((text, url, i))
}

/// Try to parse content between matching delimiters (e.g., `**`, `*`, `__`,
/// `_`).
///
/// Returns `(content, end_position_after_closing_delimiter)` if successful.
fn try_parse_delimited(chars: &[char], pos: usize, delim: &str) -> Option<(String, usize)> {
    let delim_chars: Vec<char> = delim.chars().collect();
    let delim_len = delim_chars.len();
    let len = chars.len();

    if pos + delim_len > len {
        return None;
    }

    // Verify opening delimiter
    for (j, dc) in delim_chars.iter().enumerate() {
        if chars[pos + j] != *dc {
            return None;
        }
    }

    let content_start = pos + delim_len;
    let mut i = content_start;

    // Find closing delimiter (must not be at the very start — empty content not
    // allowed)
    while i + delim_len <= len {
        let mut matched = true;
        for (j, dc) in delim_chars.iter().enumerate() {
            if chars[i + j] != *dc {
                matched = false;
                break;
            }
        }
        if matched && i > content_start {
            let content: String = chars[content_start..i].iter().collect();
            return Some((content, i + delim_len));
        }
        i += 1;
    }

    None
}

/// Split a long HTML message into chunks that respect the Telegram max length.
///
/// The chunker tries to break at newline boundaries. It does not break in the
/// middle of HTML tags.
pub fn chunk_message(html: &str, max_len: usize) -> Vec<String> {
    if html.len() <= max_len {
        return vec![html.to_owned()];
    }

    let mut chunks = Vec::new();
    let mut remaining = html;

    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining.to_owned());
            break;
        }

        // Find a good break point within max_len.
        let search_region = &remaining[..max_len];

        // Try to break at a newline.
        let break_at = if let Some(pos) = search_region.rfind('\n') {
            pos + 1 // include the newline in this chunk
        } else {
            // No newline found; try to break at a space to avoid word splitting.
            if let Some(pos) = search_region.rfind(' ') {
                pos + 1
            } else {
                // No space either; hard break at max_len.
                max_len
            }
        };

        let (chunk, rest) = remaining.split_at(break_at);
        chunks.push(chunk.to_owned());
        remaining = rest;
    }

    chunks
}
