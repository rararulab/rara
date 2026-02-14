// Copyright 2025 Crrow
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

//! Markdown chunking strategies.
//!
//! Splits a markdown document into semantic chunks, preferring heading-based
//! sections (H2 `##`). Falls back to paragraph-boundary splitting at ~1000
//! characters when no headings are present.

use crate::types::MemoryChunk;

/// Target chunk size in characters when splitting by paragraphs.
const TARGET_CHUNK_SIZE: usize = 1000;

/// Split markdown content into [`MemoryChunk`]s.
///
/// Strategy:
/// 1. If the document contains H2 headings, split at each `## ` boundary.
///    Each section (including any preamble before the first heading) becomes a
///    chunk.
/// 2. If no H2 headings are found, split at paragraph boundaries (`\n\n`)
///    trying to keep chunks close to [`TARGET_CHUNK_SIZE`] characters.
pub fn chunk_markdown(doc_id: &str, content: &str) -> Vec<MemoryChunk> {
    let sections = split_by_headings(content);

    if sections.len() > 1 {
        // Heading-based chunks.
        sections
            .into_iter()
            .enumerate()
            .filter(|(_, (_, text))| !text.trim().is_empty())
            .map(|(idx, (heading, text))| MemoryChunk {
                chunk_id:    format!("{doc_id}#{idx}"),
                doc_id:      doc_id.to_owned(),
                content:     text,
                heading,
                #[allow(clippy::cast_possible_truncation)]
                chunk_index: idx as u32,
            })
            .collect()
    } else {
        // No headings — fall back to paragraph splitting.
        let paragraphs = split_by_paragraphs(content);
        paragraphs
            .into_iter()
            .enumerate()
            .filter(|(_, text)| !text.trim().is_empty())
            .map(|(idx, text)| MemoryChunk {
                chunk_id:    format!("{doc_id}#{idx}"),
                doc_id:      doc_id.to_owned(),
                content:     text,
                heading:     None,
                #[allow(clippy::cast_possible_truncation)]
                chunk_index: idx as u32,
            })
            .collect()
    }
}

/// Split content by H2 (`## `) headings.
///
/// Returns a vec of `(Option<heading_text>, section_body)`.
fn split_by_headings(content: &str) -> Vec<(Option<String>, String)> {
    let mut sections: Vec<(Option<String>, String)> = Vec::new();
    let mut current_heading: Option<String> = None;
    let mut current_body = String::new();

    for line in content.lines() {
        if let Some(heading_text) = line.strip_prefix("## ") {
            // Flush previous section.
            if !current_body.is_empty() || current_heading.is_some() {
                sections.push((current_heading.take(), std::mem::take(&mut current_body)));
            }
            current_heading = Some(heading_text.trim().to_owned());
        } else {
            if !current_body.is_empty() {
                current_body.push('\n');
            }
            current_body.push_str(line);
        }
    }

    // Flush the last section.
    if !current_body.is_empty() || current_heading.is_some() {
        sections.push((current_heading, current_body));
    }

    sections
}

/// Split content at paragraph boundaries (`\n\n`), merging paragraphs until
/// the chunk reaches approximately [`TARGET_CHUNK_SIZE`] characters.
fn split_by_paragraphs(content: &str) -> Vec<String> {
    let paragraphs: Vec<&str> = content.split("\n\n").collect();
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();

    for para in paragraphs {
        let para = para.trim();
        if para.is_empty() {
            continue;
        }

        if !current.is_empty() && current.len() + para.len() > TARGET_CHUNK_SIZE {
            chunks.push(std::mem::take(&mut current));
        }

        if !current.is_empty() {
            current.push_str("\n\n");
        }
        current.push_str(para);
    }

    if !current.is_empty() {
        chunks.push(current);
    }

    chunks
}

/// Extract the title from markdown content.
///
/// Returns the text of the first H1 (`# `) heading, or `None` if there is no
/// H1.
pub fn extract_title(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(title) = trimmed.strip_prefix("# ") {
            let title = title.trim();
            if !title.is_empty() {
                return Some(title.to_owned());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_heading_based_chunking() {
        let content = "\
# My Doc

Preamble text.

## Section One

Content of section one.

## Section Two

Content of section two.
More content here.
";
        let chunks = chunk_markdown("test.md", content);
        assert_eq!(chunks.len(), 3);

        // Preamble
        assert!(chunks[0].heading.is_none());
        assert!(chunks[0].content.contains("Preamble text."));

        // Section One
        assert_eq!(
            chunks[1].heading.as_deref(),
            Some("Section One")
        );
        assert!(chunks[1].content.contains("Content of section one."));

        // Section Two
        assert_eq!(
            chunks[2].heading.as_deref(),
            Some("Section Two")
        );
        assert!(chunks[2].content.contains("Content of section two."));
    }

    #[test]
    fn test_paragraph_based_chunking() {
        // No H2 headings — should split by paragraphs.
        let content = "First paragraph.\n\nSecond paragraph.\n\nThird paragraph.";
        let chunks = chunk_markdown("no_headings.md", content);

        // All paragraphs are small, so they should merge into one chunk.
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].content.contains("First paragraph."));
        assert!(chunks[0].content.contains("Third paragraph."));
    }

    #[test]
    fn test_paragraph_splitting_large() {
        // Create content > TARGET_CHUNK_SIZE with paragraph boundaries.
        let para = "A".repeat(600);
        let content = format!("{para}\n\n{para}\n\n{para}");
        let chunks = chunk_markdown("large.md", &content);

        // Each paragraph is 600 chars; merging first two would be 1200 > 1000,
        // so we expect 2-3 chunks.
        assert!(
            chunks.len() >= 2,
            "expected at least 2 chunks, got {}",
            chunks.len()
        );
    }

    #[test]
    fn test_extract_title() {
        assert_eq!(
            extract_title("# Hello World\n\nSome content."),
            Some("Hello World".to_owned())
        );
        assert_eq!(extract_title("No heading here."), None);
        assert_eq!(
            extract_title("## Not H1\n# Actual Title"),
            Some("Actual Title".to_owned())
        );
    }

    #[test]
    fn test_empty_content() {
        let chunks = chunk_markdown("empty.md", "");
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_chunk_ids() {
        let content = "## A\n\nContent A\n\n## B\n\nContent B";
        let chunks = chunk_markdown("doc.md", content);
        assert_eq!(chunks[0].chunk_id, "doc.md#0");
        assert_eq!(chunks[1].chunk_id, "doc.md#1");
        assert_eq!(chunks[0].doc_id, "doc.md");
    }
}
