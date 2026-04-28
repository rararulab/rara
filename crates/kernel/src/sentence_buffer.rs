//! Accumulates streaming text deltas and emits complete sentences.
//!
//! Used by channel adapters that need sentence-level chunking for TTS
//! synthesis — feeding one sentence at a time produces natural prosody
//! while keeping latency low.

/// Accumulates text and emits complete sentences split on sentence-ending
/// punctuation (`。！？.!?\n`).
///
/// No async, no I/O — pure text segmentation. Designed to sit between an
/// LLM `TextDelta` stream and a TTS synthesizer.
#[derive(Debug, Default)]
pub struct SentenceBuffer {
    buf: String,
}

/// Characters that terminate a sentence.
const SENTENCE_ENDS: &[char] = &['。', '！', '？', '.', '!', '?', '\n'];

impl SentenceBuffer {
    /// Create an empty buffer.
    pub fn new() -> Self { Self::default() }

    /// Push a text delta. Returns any complete sentences that were formed.
    ///
    /// A sentence is delimited by any sentence-ending character
    /// (`。！？.!?\n`). The
    /// delimiter is included in the returned sentence. Consecutive delimiters
    /// (e.g. `"?!"`) are collapsed into one sentence.
    pub fn push(&mut self, delta: &str) -> Vec<String> {
        self.buf.push_str(delta);
        self.drain_sentences()
    }

    /// Drain any remaining text that hasn't been terminated by a sentence
    /// delimiter. Call this when the LLM turn ends.
    pub fn flush(&mut self) -> Option<String> {
        let rest = std::mem::take(&mut self.buf);
        let trimmed = rest.trim().to_owned();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    }

    /// Returns `true` if the buffer has no accumulated text.
    pub fn is_empty(&self) -> bool { self.buf.trim().is_empty() }

    fn drain_sentences(&mut self) -> Vec<String> {
        let mut sentences = Vec::new();

        loop {
            let pos = self.buf.find(SENTENCE_ENDS);
            let Some(byte_pos) = pos else { break };

            // Include the delimiter character.
            let end = byte_pos + self.buf[byte_pos..].chars().next().unwrap().len_utf8();

            // Skip consecutive delimiters (e.g. "?!" or "。\n").
            let mut scan = end;
            for ch in self.buf[end..].chars() {
                if SENTENCE_ENDS.contains(&ch) {
                    scan += ch.len_utf8();
                } else {
                    break;
                }
            }

            let sentence = self.buf[..scan].trim().to_owned();
            self.buf = self.buf[scan..].to_owned();

            if !sentence.is_empty() {
                sentences.push(sentence);
            }
        }

        sentences
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_sentence() {
        let mut buf = SentenceBuffer::new();
        assert!(buf.push("こんに").is_empty());
        assert!(buf.push("ちは").is_empty());
        let out = buf.push("。");
        assert_eq!(out, vec!["こんにちは。"]);
        assert!(buf.is_empty());
    }

    #[test]
    fn multiple_sentences_in_one_push() {
        let mut buf = SentenceBuffer::new();
        let out = buf.push("Hello. How are you? Fine!");
        assert_eq!(out, vec!["Hello.", "How are you?", "Fine!"]);
    }

    #[test]
    fn flush_remaining() {
        let mut buf = SentenceBuffer::new();
        buf.push("no terminator");
        assert_eq!(buf.flush(), Some("no terminator".to_owned()));
        assert!(buf.is_empty());
    }

    #[test]
    fn flush_empty() {
        let mut buf = SentenceBuffer::new();
        assert_eq!(buf.flush(), None);
    }

    #[test]
    fn consecutive_delimiters_collapsed() {
        let mut buf = SentenceBuffer::new();
        let out = buf.push("Really?! Yes.");
        assert_eq!(out, vec!["Really?!", "Yes."]);
    }

    #[test]
    fn newline_as_delimiter() {
        let mut buf = SentenceBuffer::new();
        let out = buf.push("Line one\nLine two\n");
        assert_eq!(out, vec!["Line one", "Line two"]);
    }

    #[test]
    fn chinese_mixed_punctuation() {
        let mut buf = SentenceBuffer::new();
        let out = buf.push("今天天气真好。明天呢？不知道！");
        assert_eq!(out, vec!["今天天气真好。", "明天呢？", "不知道！"]);
    }

    #[test]
    fn incremental_deltas() {
        let mut buf = SentenceBuffer::new();
        assert!(buf.push("I am ").is_empty());
        assert!(buf.push("fine").is_empty());
        let out = buf.push(". Thank you!");
        assert_eq!(out, vec!["I am fine.", "Thank you!"]);
    }

    #[test]
    fn trailing_text_after_sentence() {
        let mut buf = SentenceBuffer::new();
        let out = buf.push("Done. Now");
        assert_eq!(out, vec!["Done."]);
        assert_eq!(buf.flush(), Some("Now".to_owned()));
    }
}
