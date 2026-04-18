//! Application-layer segmentation for FTS5 indexing.
//!
//! SQLite FTS5's built-in `unicode61` tokenizer treats a run of CJK
//! characters as a single token because CJK scripts lack whitespace
//! boundaries. This collapses entire Chinese sentences into one FTS
//! token, which defeats BM25 ranking and falls back on the brute-force
//! fuzzy scorer.
//!
//! Instead of registering a custom FTS5 tokenizer (which requires
//! per-connection unsafe FFI hooks across the SQLx pool), we segment in
//! Rust with `jieba-rs` and emit space-separated tokens. The existing
//! `unicode61` tokenizer then splits on those spaces, yielding real
//! semantic tokens for both indexing and querying.
//!
//! The same function is applied symmetrically to indexed content
//! (`extract_fts_content`) and user queries (`sanitize_fts_query`),
//! keeping the token vocabulary consistent on both sides.

use std::sync::OnceLock;

use jieba_rs::Jieba;

static JIEBA: OnceLock<Jieba> = OnceLock::new();

fn jieba() -> &'static Jieba { JIEBA.get_or_init(Jieba::new) }

/// Eagerly initialize the shared Jieba instance.
///
/// Called once during kernel startup to absorb the ~200 ms dictionary
/// load off the hot path. Safe to call multiple times.
pub(crate) fn warmup() { let _ = jieba(); }

/// Segment `text` into whitespace-separated tokens suitable for
/// `unicode61` indexing.
///
/// Chinese runs are cut into semantic words; ASCII and whitespace are
/// preserved so mixed-language input works naturally. Empty or
/// whitespace-only input returns an empty string.
pub(crate) fn segment(text: &str) -> String {
    if text.chars().all(|c| !is_cjk(c)) {
        // Pure non-CJK text needs no pre-segmentation — unicode61
        // already splits it correctly.
        return text.to_owned();
    }

    let tokens = jieba().cut(text, false);
    tokens
        .into_iter()
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3400..=0x4DBF       // CJK Unified Ideographs Extension A
        | 0x4E00..=0x9FFF     // CJK Unified Ideographs
        | 0xF900..=0xFAFF     // CJK Compatibility Ideographs
        | 0x20000..=0x2FFFF   // CJK Extensions B–F
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_ascii_unchanged() {
        assert_eq!(segment("hello world"), "hello world");
    }

    #[test]
    fn chinese_is_segmented() {
        let out = segment("机器学习很强大");
        // Jieba should split into at least two real words.
        let tokens: Vec<_> = out.split_whitespace().collect();
        assert!(tokens.len() >= 2, "expected multiple tokens, got {out:?}");
        assert!(tokens.iter().any(|t| *t == "机器学习" || *t == "学习"));
    }

    #[test]
    fn mixed_language() {
        let out = segment("Rust 的 所有权 模型");
        let tokens: Vec<_> = out.split_whitespace().collect();
        assert!(tokens.contains(&"Rust"));
        assert!(tokens.contains(&"所有权"));
    }

    #[test]
    fn empty_input() {
        assert_eq!(segment(""), "");
        assert_eq!(segment("   "), "   ");
    }
}
