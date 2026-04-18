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

use std::sync::{Once, OnceLock};

use jieba_rs::Jieba;

static JIEBA: OnceLock<Jieba> = OnceLock::new();
static WARMUP_ONCE: Once = Once::new();

fn jieba() -> &'static Jieba { JIEBA.get_or_init(Jieba::new) }

/// Eagerly initialize the shared Jieba instance on a background thread.
///
/// Absorbs the ~200 ms dictionary load off the hot path. Subsequent
/// calls are no-ops — the spawn itself is also guarded so repeated
/// `TapeService::with_fts` invocations don't leak threads.
pub(crate) fn warmup() {
    WARMUP_ONCE.call_once(|| {
        std::thread::spawn(|| {
            let _ = jieba();
        });
    });
}

/// Segment `text` into whitespace-separated tokens suitable for
/// `unicode61` indexing.
///
/// Chinese runs are cut into semantic words; ASCII and whitespace pass
/// through jieba untouched so mixed-language input works naturally.
/// Input that contains no Chinese characters is returned verbatim
/// (the `unicode61` tokenizer already splits such text correctly).
pub(crate) fn segment(text: &str) -> String {
    if !text.chars().any(is_chinese_ideograph) {
        // Nothing for jieba to do — skip the dictionary load for
        // pure non-Chinese input (including empty/whitespace strings).
        return text.to_owned();
    }

    let tokens = jieba().cut(text, false);
    tokens
        .into_iter()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// True when `c` is a Chinese ideograph.
///
/// Deliberately narrow: jieba's dictionary only covers Chinese, so we
/// let Japanese/Korean fall through the `unicode61` path (suboptimal
/// but non-regressive). CJK punctuation is also excluded — the
/// surrounding ideographs will still trigger segmentation, and jieba
/// handles punctuation internally.
fn is_chinese_ideograph(c: char) -> bool {
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
    fn whitespace_only_unchanged() {
        // No Chinese → short-circuit returns input verbatim. Downstream
        // `split_whitespace` in `sanitize_fts_query` drops it.
        assert_eq!(segment(""), "");
        assert_eq!(segment("   "), "   ");
    }

    #[test]
    fn chinese_is_segmented_into_words() {
        // Jieba's default dictionary splits "机器学习" into "机器" + "学习"
        // — that's fine, BM25 now has real word tokens to work with
        // instead of one sentence-sized glob. The point of this test is
        // to assert *some* meaningful segmentation occurred, not to pin
        // a specific dictionary's boundaries.
        let tokens: Vec<String> = segment("今天学习了机器的原理")
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        assert!(
            tokens.contains(&"机器".to_owned()),
            "expected '机器' token, got {tokens:?}"
        );
        assert!(
            tokens.contains(&"学习".to_owned()),
            "expected '学习' token, got {tokens:?}"
        );
        assert!(
            tokens.len() >= 3,
            "expected multi-token split, got {tokens:?}"
        );
    }

    #[test]
    fn mixed_language() {
        let tokens: Vec<String> = segment("Rust 的所有权模型")
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        assert!(tokens.contains(&"Rust".to_owned()), "got {tokens:?}");
        assert!(tokens.contains(&"所有权".to_owned()), "got {tokens:?}");
    }
}
