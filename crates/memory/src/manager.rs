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
    sync::atomic::{AtomicBool, Ordering},
    sync::RwLock,
    sync::Arc,
    time::UNIX_EPOCH,
};

use sha2::{Digest, Sha256};
use snafu::prelude::*;
use walkdir::WalkDir;

use crate::{
    chroma::{ChromaChunk, ChromaClient},
    embedder::{Embedder, HashEmbedder},
    reranking::rerank_results,
    store::{ChunkInput, IndexedFileMeta, MemoryStore},
    store_pg::PgMemoryStore,
    store_sqlite::SqliteMemoryStore,
};

/// Unified memory error.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum MemoryError {
    #[snafu(display("I/O error: {source}"))]
    Io { source: std::io::Error },

    #[snafu(display("SQLite error: {source}"), context(false))]
    Db { source: rusqlite::Error },

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
/// This type owns the end-to-end indexing and retrieval flow:
/// 1. Incremental markdown sync.
/// 2. Chunk generation and optional embedding.
/// 3. Optional Chroma upsert/query.
/// 4. Hybrid fusion and reranking.
#[derive(Clone)]
pub struct MemoryManager {
    storage_backend: &'static str,
    memory_dir:    PathBuf,
    store:         Arc<dyn MemoryStore>,
    embedder:      Option<Arc<dyn Embedder>>,
    embeddings_enabled: Arc<AtomicBool>,
    chroma:        Arc<RwLock<Option<Arc<ChromaClient>>>>,
    chunk_chars:   usize,
    chunk_overlap: usize,
}

impl MemoryManager {
    /// Create a manager with SQLite storage.
    ///
    /// Intended for local/dev environments and fallback scenarios.
    pub fn open(memory_dir: PathBuf, db_path: PathBuf) -> MemoryResult<Self> {
        if !memory_dir.exists() {
            std::fs::create_dir_all(&memory_dir).context(IoSnafu)?;
        }

        if let Some(parent) = db_path.parent()
            && !parent.exists()
        {
            std::fs::create_dir_all(parent).context(IoSnafu)?;
        }

        let store = SqliteMemoryStore::new(db_path);
        store.ensure_schema()?;

        Ok(Self {
            storage_backend: "sqlite",
            memory_dir,
            store: Arc::new(store),
            embedder: Some(Arc::new(HashEmbedder::default())),
            embeddings_enabled: Arc::new(AtomicBool::new(true)),
            chroma: Arc::new(RwLock::new(ChromaClient::from_env().map(Arc::new))),
            chunk_chars: 1200,
            chunk_overlap: 200,
        })
    }

    /// Create a manager backed by PostgreSQL.
    ///
    /// This is the preferred production backend in the current architecture.
    pub fn open_postgres(memory_dir: PathBuf, pool: sqlx::PgPool) -> MemoryResult<Self> {
        if !memory_dir.exists() {
            std::fs::create_dir_all(&memory_dir).context(IoSnafu)?;
        }

        let store = PgMemoryStore::new(pool);
        store.ensure_schema()?;

        Ok(Self {
            storage_backend: "postgres",
            memory_dir,
            store: Arc::new(store),
            embedder: Some(Arc::new(HashEmbedder::default())),
            embeddings_enabled: Arc::new(AtomicBool::new(true)),
            chroma: Arc::new(RwLock::new(ChromaClient::from_env().map(Arc::new))),
            chunk_chars: 1200,
            chunk_overlap: 200,
        })
    }

    /// Disable embeddings and force keyword-only search.
    ///
    /// Useful for debugging and emergency degradation.
    #[allow(dead_code)]
    pub fn with_embeddings_disabled(mut self) -> Self {
        self.embedder = None;
        self
    }

    /// Active vector retrieval backend.
    pub fn vector_backend(&self) -> &'static str {
        if self.embedder.is_none() || !self.embeddings_enabled.load(Ordering::Relaxed) {
            "disabled"
        } else if self.chroma.read().ok().and_then(|g| g.clone()).is_some() {
            "chroma"
        } else {
            "sqlite"
        }
    }

    /// Active metadata/index backend.
    pub fn storage_backend(&self) -> &'static str { self.storage_backend }

    /// Apply runtime memory settings (hot-refresh on every tool invocation).
    ///
    /// This intentionally mutates only runtime behavior switches and does not
    /// rebuild the underlying storage backend.
    pub fn apply_runtime_settings(&self, memory: &rara_domain_shared::settings::model::MemorySettings) {
        self.embeddings_enabled
            .store(memory.embeddings_enabled, Ordering::Relaxed);

        let chroma = if memory.chroma_enabled {
            memory
                .chroma_url
                .clone()
                .and_then(|url| ChromaClient::new(
                    url,
                    memory.chroma_collection.clone(),
                    memory.chroma_api_key.clone(),
                ))
                .map(Arc::new)
        } else {
            None
        };

        if let Ok(mut guard) = self.chroma.write() {
            *guard = chroma;
        }
    }

    /// Run incremental sync from markdown files into the configured store.
    ///
    /// Files are selected by `(path, mtime, size, hash)` checks and only
    /// changed files are re-indexed.
    pub async fn sync(&self) -> MemoryResult<SyncStats> {
        let memory_dir = self.memory_dir.clone();
        let store = Arc::clone(&self.store);
        let embedder = self.embedder.clone();
        let embeddings_enabled = self.embeddings_enabled.load(Ordering::Relaxed);
        let chroma = self.chroma.read().ok().and_then(|g| g.clone());
        let chroma_enabled = chroma.is_some();
        let chunk_chars = self.chunk_chars;
        let chunk_overlap = self.chunk_overlap;

        let sync_result: MemoryResult<(SyncStats, Vec<ChromaChunk>)> =
            tokio::task::spawn_blocking(move || -> MemoryResult<(SyncStats, Vec<ChromaChunk>)> {
            let indexed = store.list_files()?;
            let indexed_map: HashMap<String, IndexedFileMeta> = indexed
                .into_iter()
                .map(|meta| (meta.path.clone(), meta))
                .collect();

            let mut seen_paths = HashSet::new();
            let mut changed_files = Vec::new();

            for entry in WalkDir::new(&memory_dir)
                .into_iter()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_type().is_file())
            {
                let path = entry.path();
                if !is_markdown_file(path) {
                    continue;
                }

                let relative = relative_path(&memory_dir, path);
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
                changed_files.push((relative, hash, mtime, size, content));
            }

            let mut stats = SyncStats::default();
            let mut chroma_chunks = Vec::new();

            for (relative, hash, mtime, size, content) in changed_files {
                let mut chunks = chunk_text(&content, chunk_chars, chunk_overlap);
                if embeddings_enabled && let Some(embedder_ref) = &embedder {
                    for chunk in &mut chunks {
                        let text_hash = text_hash(&chunk.content);
                        let embedding = store.get_cached_embedding(
                            embedder_ref.provider(),
                            embedder_ref.model(),
                            &text_hash,
                        )?;

                        let embedding = if let Some(cached) = embedding {
                            cached
                        } else {
                            let fresh = embedder_ref.embed(&chunk.content)?;
                            store.put_cached_embedding(
                                embedder_ref.provider(),
                                embedder_ref.model(),
                                &text_hash,
                                &fresh,
                            )?;
                            fresh
                        };

                        chunk.embedding = Some(embedding);
                    }
                }

                stats.total_chunks += chunks.len();
                store.upsert_file_chunks(&relative, &hash, mtime, size, &chunks)?;
                stats.indexed_files += 1;

                // Build Chroma upsert payload only when vector features are
                // active and a Chroma backend is configured.
                if embeddings_enabled && embedder.is_some() && chroma_enabled {
                    let stored = store.list_embedded_chunks_by_path(&relative)?;
                    for row in stored {
                        chroma_chunks.push(ChromaChunk {
                            id: row.chunk_id.to_string(),
                            document: row.content,
                            embedding: row.embedding,
                            path: row.path,
                            chunk_index: row.chunk_index,
                        });
                    }
                }
            }

            let to_delete = indexed_map
                .keys()
                .filter(|path| !seen_paths.contains(*path))
                .cloned()
                .collect::<Vec<_>>();

            if !to_delete.is_empty() {
                store.delete_files(&to_delete)?;
                stats.deleted_files = to_delete.len();
            }

                Ok((stats, chroma_chunks))
            })
            .await
            .context(TaskJoinSnafu)?;
        let (stats, chroma_chunks) = sync_result?;

        // Chroma is best-effort: retrieval still works with local fallback if
        // remote upsert fails.
        if let Some(chroma) = chroma
            && let Err(err) = chroma.upsert_chunks(&chroma_chunks).await
        {
            tracing::warn!(error = %err, "failed to upsert chunks to chroma, fallback remains available");
        }

        Ok(stats)
    }

    /// Search memory with automatic strategy selection.
    ///
    /// Strategy order:
    /// 1. Hybrid via Chroma + keyword (if enabled and reachable).
    /// 2. Hybrid via local vectors + keyword.
    /// 3. Keyword-only fallback.
    pub async fn search(&self, query: &str, limit: usize) -> MemoryResult<Vec<SearchResult>> {
        let store = Arc::clone(&self.store);
        let embedder = self.embedder.clone();
        let chroma = self.chroma.read().ok().and_then(|g| g.clone());
        let embeddings_enabled = self.embeddings_enabled.load(Ordering::Relaxed);
        let query = query.to_owned();
        let limit = limit.clamp(1, 50);

        if embeddings_enabled && let Some(embedder_ref) = embedder {
            if let Some(chroma_ref) = chroma {
                let query_embedding = embedder_ref.embed(&query)?;
                let chroma_hits = chroma_ref.query(&query_embedding, (limit * 4).max(20)).await;
                if let Ok(hits) = chroma_hits {
                    return tokio::task::spawn_blocking(move || {
                        hybrid_search_with_chroma_hits(&*store, &query, limit, hits)
                    })
                    .await
                    .context(TaskJoinSnafu)?;
                }
            }

            return tokio::task::spawn_blocking(move || {
                hybrid_search_blocking(&*store, &*embedder_ref, &query, limit)
            })
            .await
            .context(TaskJoinSnafu)?;
        }

        tokio::task::spawn_blocking(move || keyword_only_search_blocking(&*store, &query, limit))
            .await
            .context(TaskJoinSnafu)?
    }

    /// Fetch full chunk by id.
    pub async fn get_chunk(&self, chunk_id: i64) -> MemoryResult<Option<ChunkDetail>> {
        let store = Arc::clone(&self.store);
        tokio::task::spawn_blocking(move || store.get_chunk(chunk_id))
            .await
            .context(TaskJoinSnafu)?
    }
}

fn keyword_only_search_blocking(
    store: &dyn MemoryStore,
    query: &str,
    limit: usize,
) -> MemoryResult<Vec<SearchResult>> {
    // Convert backend ranking to reciprocal-rank style score so result shape
    // is consistent with hybrid flows.
    let rows = store.keyword_search(query, limit)?;
    Ok(rows
        .into_iter()
        .enumerate()
        .map(|(rank, row)| SearchResult {
            chunk_id: row.chunk_id,
            path: row.path,
            chunk_index: row.chunk_index,
            snippet: make_snippet(&row.content, 220),
            score: reciprocal_rank(rank),
        })
        .collect())
}

fn hybrid_search_blocking(
    store: &dyn MemoryStore,
    embedder: &dyn Embedder,
    query: &str,
    limit: usize,
) -> MemoryResult<Vec<SearchResult>> {
    // Retrieve a broader candidate set from each channel, then fuse.
    let keyword_limit = (limit * 4).max(20);
    let vector_limit = (limit * 4).max(20);

    let keyword_rows = store.keyword_search(query, keyword_limit)?;

    let query_hash = text_hash(query);
    let query_embedding = if let Some(cached) =
        store.get_cached_embedding(embedder.provider(), embedder.model(), &query_hash)?
    {
        cached
    } else {
        let fresh = embedder.embed(query)?;
        store.put_cached_embedding(
            embedder.provider(),
            embedder.model(),
            &query_hash,
            &fresh,
        )?;
        fresh
    };

    let embedded_rows = store.list_embedded_chunks(5000)?;
    let mut vector_rows = embedded_rows
        .into_iter()
        .map(|row| {
            let sim = cosine_similarity(&query_embedding, &row.embedding);
            (sim, row)
        })
        .collect::<Vec<_>>();

    vector_rows.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    // Reciprocal-rank fusion with channel-specific weights.
    let mut fused: HashMap<i64, SearchResult> = HashMap::new();

    for (rank, row) in keyword_rows.into_iter().enumerate() {
        let kw_score = reciprocal_rank(rank);
        let entry = fused.entry(row.chunk_id).or_insert_with(|| SearchResult {
            chunk_id: row.chunk_id,
            path: row.path.clone(),
            chunk_index: row.chunk_index,
            snippet: make_snippet(&row.content, 220),
            score: 0.0,
        });
        entry.score += kw_score * 0.65;
    }

    for (rank, (sim, row)) in vector_rows.into_iter().take(vector_limit).enumerate() {
        let vec_rank_score = reciprocal_rank(rank);
        let sim_weight = ((sim + 1.0) / 2.0).clamp(0.0, 1.0) as f64;

        let entry = fused.entry(row.chunk_id).or_insert_with(|| SearchResult {
            chunk_id: row.chunk_id,
            path: row.path.clone(),
            chunk_index: row.chunk_index,
            snippet: make_snippet(&row.content, 220),
            score: 0.0,
        });
        entry.score += vec_rank_score * sim_weight * 0.35;
    }

    // Keep extra candidates for lightweight reranking.
    let mut candidates = fused.into_values().collect::<Vec<_>>();
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    candidates.truncate((limit * 3).max(limit));

    Ok(rerank_results(query, candidates, limit))
}

fn hybrid_search_with_chroma_hits(
    store: &dyn MemoryStore,
    query: &str,
    limit: usize,
    chroma_hits: Vec<crate::chroma::ChromaHit>,
) -> MemoryResult<Vec<SearchResult>> {
    // Same fusion strategy as local hybrid, but vector candidates come from
    // Chroma instead of the local embedded chunk scan.
    let keyword_limit = (limit * 4).max(20);
    let keyword_rows = store.keyword_search(query, keyword_limit)?;

    let mut fused: HashMap<i64, SearchResult> = HashMap::new();

    for (rank, row) in keyword_rows.into_iter().enumerate() {
        let kw_score = reciprocal_rank(rank);
        let entry = fused.entry(row.chunk_id).or_insert_with(|| SearchResult {
            chunk_id: row.chunk_id,
            path: row.path.clone(),
            chunk_index: row.chunk_index,
            snippet: make_snippet(&row.content, 220),
            score: 0.0,
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

    Ok(rerank_results(query, candidates, limit))
}

fn reciprocal_rank(rank: usize) -> f64 {
    1.0 / (rank as f64 + 1.0)
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || b.is_empty() || a.len() != b.len() {
        return 0.0;
    }

    let mut dot = 0.0_f32;
    let mut na = 0.0_f32;
    let mut nb = 0.0_f32;

    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }

    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }

    dot / (na.sqrt() * nb.sqrt())
}

fn text_hash(text: &str) -> String { format!("{:x}", Sha256::digest(text.as_bytes())) }

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
        let chunk = chars[start..end].iter().collect::<String>().trim().to_owned();

        if !chunk.is_empty() {
            out.push(ChunkInput {
                chunk_index,
                content: chunk,
                embedding: None,
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
    use sqlx::postgres::PgPoolOptions;
    use tempfile::tempdir;

    use super::*;

    #[tokio::test]
    async fn sync_and_search_markdown_files() {
        let temp = tempdir().expect("tempdir");
        let memory_dir = temp.path().join("memory");
        let db_file = temp.path().join("memory.db");
        std::fs::create_dir_all(&memory_dir).expect("create memory dir");

        std::fs::write(
            memory_dir.join("MEMORY.md"),
            "# User profile\nPrefers Rust backend roles in Tokyo.",
        )
        .expect("write markdown");

        let manager = MemoryManager::open(memory_dir, db_file).expect("open manager");
        let stats = manager.sync().await.expect("sync");
        assert_eq!(stats.indexed_files, 1);

        let hits = manager.search("Rust Tokyo", 5).await.expect("search");
        assert!(!hits.is_empty());

        let chunk = manager
            .get_chunk(hits[0].chunk_id)
            .await
            .expect("get chunk");
        assert!(chunk.is_some());
    }

    #[test]
    fn chunking_produces_overlapping_windows() {
        let text = "a".repeat(3000);
        let chunks = chunk_text(&text, 1000, 100);
        assert!(chunks.len() >= 3);
        assert_eq!(chunks[0].chunk_index, 0);
        assert_eq!(chunks[1].chunk_index, 1);
    }

    #[tokio::test]
    async fn pg_chroma_smoke_when_env_configured() {
        let Some(database_url) = std::env::var("MEMORY_SMOKE_PG_URL").ok() else {
            return;
        };
        let Some(chroma_url) = std::env::var("MEMORY_SMOKE_CHROMA_URL").ok() else {
            return;
        };

        let pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(&database_url)
            .await
            .expect("connect pg");

        let temp = tempdir().expect("tempdir");
        let memory_dir = temp.path().join("memory");
        std::fs::create_dir_all(&memory_dir).expect("create memory dir");
        std::fs::write(
            memory_dir.join("MEMORY.md"),
            "# Team memory\nChroma smoke test for rust backend job assistant.",
        )
        .expect("write markdown");

        let manager =
            MemoryManager::open_postgres(memory_dir, pool).expect("open postgres manager");

        let mut runtime_memory =
            rara_domain_shared::settings::model::MemorySettings::default();
        runtime_memory.chroma_enabled = true;
        runtime_memory.chroma_url = Some(chroma_url);
        runtime_memory.chroma_collection = std::env::var("MEMORY_SMOKE_CHROMA_COLLECTION")
            .ok()
            .or_else(|| Some("job-memory-smoke".to_owned()));
        runtime_memory.chroma_api_key =
            std::env::var("MEMORY_SMOKE_CHROMA_API_KEY").ok();
        manager.apply_runtime_settings(&runtime_memory);

        manager.sync().await.expect("sync");
        let hits = manager
            .search("chroma rust assistant", 5)
            .await
            .expect("search");
        assert!(!hits.is_empty());
        assert_eq!(manager.storage_backend(), "postgres");
    }
}
