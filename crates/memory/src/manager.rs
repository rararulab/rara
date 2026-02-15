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

//! Memory manager that orchestrates sync/chunk/index/search.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::UNIX_EPOCH,
};

use sha2::{Digest, Sha256};
use snafu::prelude::*;
use walkdir::WalkDir;

use crate::{
    chroma::{ChromaChunk, ChromaClient},
    reranking::rerank_results,
    store::ChunkInput,
    store_pg::PgMemoryStore,
};

/// Unified memory error.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum MemoryError {
    #[snafu(display("I/O error: {source}"))]
    Io { source: std::io::Error },

    #[snafu(display("task join error: {source}"))]
    TaskJoin { source: tokio::task::JoinError },

    #[snafu(display("database error: {source}"), context(false))]
    Sqlx { source: sqlx::Error },

    #[snafu(display("{message}"))]
    Other { message: String },
}

/// Result alias for memory operations.
pub type MemoryResult<T> = Result<T, MemoryError>;

/// Search hit returned to tools.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub chunk_id:    i64,
    pub path:        String,
    pub chunk_index: i64,
    pub snippet:     String,
    /// Final fused score after hybrid merge + reranking.
    pub score:       f64,
}

/// Full chunk payload for `memory_get`.
#[derive(Debug, Clone)]
pub struct ChunkDetail {
    pub chunk_id:    i64,
    pub path:        String,
    pub chunk_index: i64,
    pub content:     String,
}

/// Sync statistics.
#[derive(Debug, Clone, Default)]
pub struct SyncStats {
    pub indexed_files: usize,
    pub deleted_files: usize,
    pub total_chunks:  usize,
}

/// High-level memory orchestrator.
///
/// Owns the end-to-end indexing and retrieval flow:
/// 1. Incremental markdown sync (PG metadata + chunks).
/// 2. Chroma upsert (server-side embeddings).
/// 3. Hybrid search (Chroma vector + PG keyword fusion).
/// 4. Token-overlap reranking.
#[derive(Clone)]
pub struct MemoryManager {
    memory_dir:    PathBuf,
    store:         Arc<PgMemoryStore>,
    chroma:        Arc<ChromaClient>,
    chunk_chars:   usize,
    chunk_overlap: usize,
}

impl MemoryManager {
    /// Create a new memory manager backed by PostgreSQL + Chroma.
    pub fn new(
        memory_dir: PathBuf,
        pool: sqlx::PgPool,
        chroma: ChromaClient,
    ) -> MemoryResult<Self> {
        if !memory_dir.exists() {
            std::fs::create_dir_all(&memory_dir).context(IoSnafu)?;
        }

        Ok(Self {
            memory_dir,
            store: Arc::new(PgMemoryStore::new(pool)),
            chroma: Arc::new(chroma),
            chunk_chars: 1200,
            chunk_overlap: 200,
        })
    }

    /// Run incremental sync from markdown files into PG + Chroma.
    pub async fn sync(&self) -> MemoryResult<SyncStats> {
        // Phase 1: async — fetch current index from PG.
        let indexed = self.store.list_files().await?;
        let indexed_map: HashMap<String, _> = indexed
            .into_iter()
            .map(|meta| (meta.path.clone(), meta))
            .collect();

        // Phase 2: blocking — filesystem walk, diff, chunk.
        let memory_dir = self.memory_dir.clone();
        let chunk_chars = self.chunk_chars;
        let chunk_overlap = self.chunk_overlap;

        let (changed, to_delete) = tokio::task::spawn_blocking(move || {
            scan_filesystem(&memory_dir, &indexed_map, chunk_chars, chunk_overlap)
        })
        .await
        .context(TaskJoinSnafu)??;

        // Phase 3: async — upsert changed files + delete stale ones.
        let mut stats = SyncStats::default();
        let mut chroma_chunks = Vec::new();

        for change in &changed {
            self.store
                .upsert_file_chunks(
                    &change.relative,
                    &change.hash,
                    change.mtime,
                    change.size,
                    &change.chunks,
                )
                .await?;
            stats.indexed_files += 1;
            stats.total_chunks += change.chunks.len();

            for (idx, chunk) in change.chunks.iter().enumerate() {
                chroma_chunks.push(ChromaChunk {
                    id:          format!("{}:{idx}", change.relative),
                    document:    chunk.content.clone(),
                    path:        change.relative.clone(),
                    chunk_index: chunk.chunk_index,
                });
            }
        }

        if !to_delete.is_empty() {
            self.store.delete_files(&to_delete).await?;
            stats.deleted_files = to_delete.len();
        }

        // Phase 4: async — Chroma upsert.
        if !chroma_chunks.is_empty() {
            self.chroma.upsert_chunks(&chroma_chunks).await?;
        }

        Ok(stats)
    }

    /// Search memory using hybrid Chroma vector + PG keyword fusion.
    pub async fn search(&self, query: &str, limit: usize) -> MemoryResult<Vec<SearchResult>> {
        let limit = limit.clamp(1, 50);
        let fetch_limit = (limit * 4).max(20);

        // Parallel: PG keyword + Chroma vector.
        let (keyword_rows, chroma_hits) = tokio::try_join!(
            self.store.keyword_search(query, fetch_limit),
            self.chroma.query(query, fetch_limit),
        )?;

        Ok(hybrid_fuse(query, limit, keyword_rows, chroma_hits))
    }

    /// Fetch full chunk by id.
    pub async fn get_chunk(&self, chunk_id: i64) -> MemoryResult<Option<ChunkDetail>> {
        self.store.get_chunk(chunk_id).await
    }
}

// ---------------------------------------------------------------------------
// Filesystem scan (blocking)
// ---------------------------------------------------------------------------

struct FileChange {
    relative: String,
    hash:     String,
    mtime:    i64,
    size:     i64,
    chunks:   Vec<ChunkInput>,
}

/// Walk `memory_dir`, compare with `indexed_map`, and return changed files
/// (with pre-computed chunks) plus a list of stale paths to delete.
fn scan_filesystem(
    memory_dir: &Path,
    indexed_map: &HashMap<String, crate::store::IndexedFileMeta>,
    chunk_chars: usize,
    chunk_overlap: usize,
) -> MemoryResult<(Vec<FileChange>, Vec<String>)> {
    let mut seen_paths = HashSet::new();
    let mut changes = Vec::new();

    for entry in WalkDir::new(memory_dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
    {
        let path = entry.path();
        if !is_markdown_file(path) {
            continue;
        }

        let relative = relative_path(memory_dir, path);
        seen_paths.insert(relative.clone());

        let metadata = std::fs::metadata(path).context(IoSnafu)?;
        let mtime = metadata
            .modified()
            .context(IoSnafu)?
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let size = metadata.len() as i64;

        if let Some(existing) = indexed_map.get(&relative)
            && existing.mtime == mtime
            && existing.size == size
        {
            continue;
        }

        let bytes = std::fs::read(path).context(IoSnafu)?;
        let hash = format!("{:x}", Sha256::digest(&bytes));

        if let Some(existing) = indexed_map.get(&relative)
            && existing.hash == hash
        {
            continue;
        }

        let content = String::from_utf8_lossy(&bytes).to_string();
        let chunks = chunk_text(&content, chunk_chars, chunk_overlap);
        changes.push(FileChange {
            relative,
            hash,
            mtime,
            size,
            chunks,
        });
    }

    let to_delete = indexed_map
        .keys()
        .filter(|path| !seen_paths.contains(*path))
        .cloned()
        .collect::<Vec<_>>();

    Ok((changes, to_delete))
}

// ---------------------------------------------------------------------------
// Hybrid fusion (pure CPU)
// ---------------------------------------------------------------------------

fn hybrid_fuse(
    query: &str,
    limit: usize,
    keyword_rows: Vec<crate::store::MemorySearchRow>,
    chroma_hits: Vec<crate::chroma::ChromaHit>,
) -> Vec<SearchResult> {
    let mut fused: HashMap<i64, SearchResult> = HashMap::new();

    for (rank, row) in keyword_rows.into_iter().enumerate() {
        let kw_score = reciprocal_rank(rank);
        let entry = fused.entry(row.chunk_id).or_insert_with(|| SearchResult {
            chunk_id:    row.chunk_id,
            path:        row.path.clone(),
            chunk_index: row.chunk_index,
            snippet:     make_snippet(&row.content, 220),
            score:       0.0,
        });
        entry.score += kw_score * 0.65;
    }

    for (rank, hit) in chroma_hits.into_iter().enumerate() {
        let Ok(chunk_id) = hit.id.parse::<i64>() else {
            continue;
        };

        let vec_rank_score = reciprocal_rank(rank);
        let sim_weight = hit.score.clamp(0.0, 1.0);

        let entry = fused.entry(chunk_id).or_insert_with(|| SearchResult {
            chunk_id,
            path: hit.path.clone(),
            chunk_index: hit.chunk_index,
            snippet: make_snippet(&hit.document, 220),
            score: 0.0,
        });
        entry.score += vec_rank_score * sim_weight * 0.35;
    }

    let mut candidates = fused.into_values().collect::<Vec<_>>();
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    candidates.truncate((limit * 3).max(limit));

    rerank_results(query, candidates, limit)
}

fn reciprocal_rank(rank: usize) -> f64 { 1.0 / (rank as f64 + 1.0) }

fn is_markdown_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
}

fn relative_path(base: &Path, path: &Path) -> String {
    path.strip_prefix(base).map_or_else(
        |_| path.to_string_lossy().to_string(),
        |relative| relative.to_string_lossy().to_string(),
    )
}

fn chunk_text(input: &str, chunk_chars: usize, overlap_chars: usize) -> Vec<ChunkInput> {
    let normalized = input.trim();
    if normalized.is_empty() {
        return Vec::new();
    }

    let chars: Vec<char> = normalized.chars().collect();
    let mut out = Vec::new();

    let mut start = 0_usize;
    let mut chunk_index = 0_i64;
    while start < chars.len() {
        let end = (start + chunk_chars).min(chars.len());
        let chunk = chars[start..end]
            .iter()
            .collect::<String>()
            .trim()
            .to_owned();

        if !chunk.is_empty() {
            out.push(ChunkInput {
                chunk_index,
                content: chunk,
            });
            chunk_index += 1;
        }

        if end == chars.len() {
            break;
        }

        let next_start = end.saturating_sub(overlap_chars);
        if next_start <= start {
            break;
        }
        start = next_start;
    }

    out
}

fn make_snippet(content: &str, max_chars: usize) -> String {
    let mut snippet = content.chars().take(max_chars).collect::<String>();
    if content.chars().count() > max_chars {
        snippet.push_str("...");
    }
    snippet
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunking_produces_overlapping_windows() {
        let text = "a".repeat(3000);
        let chunks = chunk_text(&text, 1000, 100);
        assert!(chunks.len() >= 3);
        assert_eq!(chunks[0].chunk_index, 0);
        assert_eq!(chunks[1].chunk_index, 1);
    }

    #[test]
    fn chunking_single_small_file() {
        let text = "hello world";
        let chunks = chunk_text(text, 1200, 200);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].content, "hello world");
    }

    #[test]
    fn chunking_empty_input() {
        let chunks = chunk_text("", 1200, 200);
        assert!(chunks.is_empty());
    }

    #[test]
    fn snippet_truncation() {
        let text = "a".repeat(300);
        let snippet = make_snippet(&text, 220);
        assert!(snippet.ends_with("..."));
        assert!(snippet.len() < 300);
    }
}
