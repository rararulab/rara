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

//! Snippet generation for session full-text search results.
//!
//! Given the plain text of a matched message and the user's query, produce
//! an HTML-escaped snippet with the first matched term wrapped in
//! `<mark>…</mark>`. The snippet is bounded to a short window around the
//! match so the UI can render it inline in a search result list.

/// Maximum characters kept on either side of a match when the source text
/// exceeds [`FULL_TEXT_THRESHOLD`].
const SNIPPET_CONTEXT_CHARS: usize = 40;

/// Texts shorter than this are returned in full (still escaped and
/// `<mark>`-wrapped) without ellipses — readers benefit more from seeing
/// the whole short message than a trimmed window.
const FULL_TEXT_THRESHOLD: usize = 100;

/// Fallback prefix length when no query token can be located in the text
/// (e.g. FTS matched on a CJK compound the plain-text scanner cannot
/// align to without a tokenizer).
const FALLBACK_PREFIX_CHARS: usize = 80;

/// Minimal HTML escape for `& < > " '`.
///
/// Stay self-contained — the `html_escape` crate is not a workspace
/// dependency and pulling it in for five substitutions is not worth a
/// version bump.
pub fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            other => out.push(other),
        }
    }
    out
}

/// Build a search-result snippet for `text` highlighting the first
/// whitespace-split token from `query` that appears (case-insensitively)
/// in `text`.
///
/// Returns an empty string when `text` is empty. When no token can be
/// located in the plain text, returns the first `FALLBACK_PREFIX_CHARS`
/// escaped characters without a `<mark>` wrap — the FTS layer may match
/// on tokenized CJK compounds that we cannot realign here without
/// re-tokenizing.
pub fn build_snippet(text: &str, query: &str) -> String {
    if text.is_empty() {
        return String::new();
    }

    let tokens: Vec<&str> = query.split_whitespace().collect();
    let match_range = tokens
        .iter()
        .find_map(|tok| find_case_insensitive(text, tok));

    let Some((start, end)) = match_range else {
        return fallback_prefix(text);
    };

    // Short texts are returned in full with the match highlighted.
    if text.chars().count() <= FULL_TEXT_THRESHOLD {
        return wrap_match(text, start, end);
    }

    // Take ±SNIPPET_CONTEXT_CHARS characters around the match on a char
    // boundary (`start`/`end` are byte indices, so we walk chars).
    let (window_start, leading_ellipsis) = window_start(text, start);
    let (window_end, trailing_ellipsis) = window_end(text, end);

    let before = &text[window_start..start];
    let matched = &text[start..end];
    let after = &text[end..window_end];

    let mut out = String::new();
    if leading_ellipsis {
        out.push('…');
    }
    out.push_str(&escape_html(before));
    out.push_str("<mark>");
    out.push_str(&escape_html(matched));
    out.push_str("</mark>");
    out.push_str(&escape_html(after));
    if trailing_ellipsis {
        out.push('…');
    }
    out
}

/// Locate `needle` inside `haystack` case-insensitively, returning the
/// byte range `[start, end)` of the match in `haystack`. The match length
/// is taken from `needle.len()` mapped back to char boundaries in the
/// original string to keep `<mark>` wrapping aligned with UTF-8
/// boundaries.
fn find_case_insensitive(haystack: &str, needle: &str) -> Option<(usize, usize)> {
    if needle.is_empty() {
        return None;
    }
    let lower_hay = haystack.to_lowercase();
    let lower_needle = needle.to_lowercase();
    let start = lower_hay.find(&lower_needle)?;
    // `to_lowercase` can change length for some codepoints; clamp the
    // end to a char boundary in the original string by re-walking from
    // `start` for `needle.chars().count()` characters.
    let needle_char_count = lower_needle.chars().count();
    let end = haystack[start..]
        .char_indices()
        .nth(needle_char_count)
        .map(|(offset, _)| start + offset)
        .unwrap_or(haystack.len());
    Some((start, end))
}

/// Wrap the `[start, end)` byte range in `<mark>…</mark>` against the
/// full (escaped) text. Used for short texts where no trimming applies.
fn wrap_match(text: &str, start: usize, end: usize) -> String {
    let before = &text[..start];
    let matched = &text[start..end];
    let after = &text[end..];
    let mut out = String::new();
    out.push_str(&escape_html(before));
    out.push_str("<mark>");
    out.push_str(&escape_html(matched));
    out.push_str("</mark>");
    out.push_str(&escape_html(after));
    out
}

/// Fallback when no substring from `query` could be located in `text`.
fn fallback_prefix(text: &str) -> String {
    let end = text
        .char_indices()
        .nth(FALLBACK_PREFIX_CHARS)
        .map(|(idx, _)| idx)
        .unwrap_or(text.len());
    let truncated = end < text.len();
    let mut out = escape_html(&text[..end]);
    if truncated {
        out.push('…');
    }
    out
}

/// Walk backwards from `start` up to [`SNIPPET_CONTEXT_CHARS`] chars,
/// returning the resulting byte index and whether a leading ellipsis is
/// needed.
fn window_start(text: &str, start: usize) -> (usize, bool) {
    let before = &text[..start];
    let char_count = before.chars().count();
    if char_count <= SNIPPET_CONTEXT_CHARS {
        return (0, false);
    }
    let skip = char_count - SNIPPET_CONTEXT_CHARS;
    let idx = before
        .char_indices()
        .nth(skip)
        .map(|(i, _)| i)
        .unwrap_or(start);
    (idx, true)
}

/// Walk forward from `end` up to [`SNIPPET_CONTEXT_CHARS`] chars,
/// returning the resulting byte index and whether a trailing ellipsis is
/// needed.
fn window_end(text: &str, end: usize) -> (usize, bool) {
    let after = &text[end..];
    let char_count = after.chars().count();
    if char_count <= SNIPPET_CONTEXT_CHARS {
        return (text.len(), false);
    }
    let idx = after
        .char_indices()
        .nth(SNIPPET_CONTEXT_CHARS)
        .map(|(i, _)| end + i)
        .unwrap_or(text.len());
    (idx, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_handles_all_five_specials() {
        assert_eq!(
            escape_html("a & b < c > d \" e ' f"),
            "a &amp; b &lt; c &gt; d &quot; e &#x27; f"
        );
    }

    #[test]
    fn empty_input_returns_empty() {
        assert_eq!(build_snippet("", "anything"), "");
    }

    #[test]
    fn no_match_returns_escaped_prefix() {
        let text = "hello world";
        let got = build_snippet(text, "zzz");
        assert_eq!(got, "hello world");
        assert!(!got.contains("<mark>"));
    }

    #[test]
    fn no_match_long_text_is_truncated() {
        let text = "a".repeat(200);
        let got = build_snippet(&text, "zzz");
        assert!(got.ends_with('…'));
        assert!(!got.contains("<mark>"));
    }

    #[test]
    fn short_text_returns_full_with_mark() {
        let got = build_snippet("hello world", "world");
        assert_eq!(got, "hello <mark>world</mark>");
    }

    #[test]
    fn match_at_start_of_long_text() {
        let mut text = String::from("rustlang ");
        text.push_str(&"x".repeat(200));
        let got = build_snippet(&text, "rustlang");
        assert!(got.starts_with("<mark>rustlang</mark>"));
        assert!(got.ends_with('…'));
    }

    #[test]
    fn match_mid_long_text_has_both_ellipses() {
        let mut text = String::new();
        text.push_str(&"a".repeat(200));
        text.push_str("needle");
        text.push_str(&"b".repeat(200));
        let got = build_snippet(&text, "needle");
        assert!(got.starts_with('…'));
        assert!(got.ends_with('…'));
        assert!(got.contains("<mark>needle</mark>"));
    }

    #[test]
    fn html_is_escaped_before_wrapping() {
        let got = build_snippet("<script>alert('x')</script>", "alert");
        assert!(
            got.contains("&lt;script&gt;"),
            "expected escaped opening tag, got: {got}"
        );
        assert!(
            got.contains("<mark>alert</mark>"),
            "expected unescaped mark wrap, got: {got}"
        );
        assert!(
            !got.contains("<script>"),
            "raw <script> must not appear in output: {got}"
        );
    }

    #[test]
    fn match_is_case_insensitive() {
        let got = build_snippet("Hello World", "WORLD");
        assert_eq!(got, "Hello <mark>World</mark>");
    }

    #[test]
    fn first_whitespace_token_matches_first() {
        // Both "foo" and "bar" appear; the scanner walks tokens in query
        // order, so the first query token that resolves to a hit wins —
        // independent of where each term sits in the source text.
        let got = build_snippet("alpha bar beta foo", "foo bar");
        assert!(
            got.contains("<mark>foo</mark>"),
            "expected first query token to win, got: {got}"
        );
    }

    #[test]
    fn cjk_text_is_handled_on_char_boundaries() {
        let got = build_snippet("你好世界，rust 很好", "rust");
        assert!(got.contains("<mark>rust</mark>"));
    }

    #[test]
    fn empty_query_falls_back_to_prefix() {
        let got = build_snippet("hello world", "");
        assert_eq!(got, "hello world");
        assert!(!got.contains("<mark>"));
    }
}
