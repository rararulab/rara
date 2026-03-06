# Knowledge Layer Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a structured long-term memory system (Knowledge Layer) on top of the existing tape, with automatic extraction, embedding search (usearch), and category markdown files.

**Architecture:** Three layers — tape (existing, untouched) → memory items (SQLite + usearch) → category files (markdown on disk). Session-end event triggers async LLM extraction. MemoryTool exposes search/read to the agent.

**Tech Stack:** Rust, sqlx (SQLite), usearch, OpenAI embeddings API (via reqwest), serde, tokio

**Design doc:** `docs/plans/2026-03-06-knowledge-layer-design.md`

---

## Task 1: SQLite Migration — `memory_items` Table

**Files:**
- Create: `crates/rara-model/migrations/{TIMESTAMP}_knowledge_memory_items.up.sql`
- Create: `crates/rara-model/migrations/{TIMESTAMP}_knowledge_memory_items.down.sql`

**Step 1: Create migration files**

```bash
just migrate-add knowledge_memory_items
```

**Step 2: Write the up migration**

Edit the generated `.up.sql`:

```sql
CREATE TABLE memory_items (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    username        TEXT NOT NULL,
    content         TEXT NOT NULL,
    memory_type     TEXT NOT NULL,
    category        TEXT NOT NULL,
    source_tape     TEXT,
    source_entry_id INTEGER,
    embedding       BLOB,
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_memory_items_username ON memory_items(username);
CREATE INDEX idx_memory_items_category ON memory_items(username, category);

CREATE TRIGGER set_memory_items_updated_at AFTER UPDATE ON memory_items
BEGIN
    UPDATE memory_items SET updated_at = datetime('now') WHERE id = NEW.id;
END;
```

**Step 3: Write the down migration**

Edit the generated `.down.sql`:

```sql
DROP TRIGGER IF EXISTS set_memory_items_updated_at;
DROP INDEX IF EXISTS idx_memory_items_category;
DROP INDEX IF EXISTS idx_memory_items_username;
DROP TABLE IF EXISTS memory_items;
```

**Step 4: Run migration**

```bash
just migrate-run
```

**Step 5: Commit**

```bash
git add crates/rara-model/migrations/
git commit -m "feat(model): add memory_items table for knowledge layer (#N)"
```

---

## Task 2: Add `usearch` Dependency

**Files:**
- Modify: `crates/kernel/Cargo.toml`

**Step 1: Add usearch to kernel dependencies**

Add under `[dependencies]`:

```toml
usearch = "2"
```

**Step 2: Verify it compiles**

```bash
cargo check -p rara-kernel
```

**Step 3: Commit**

```bash
git add crates/kernel/Cargo.toml
git commit -m "chore(kernel): add usearch dependency for knowledge layer (#N)"
```

---

## Task 3: Knowledge Config

**Files:**
- Create: `crates/kernel/src/memory/knowledge/config.rs`

**Step 1: Write the config struct**

```rust
use bon::Builder;
use serde::Deserialize;

/// Configuration for the Knowledge Layer.
///
/// Loaded from `memory.knowledge` section in config.yaml.
/// All fields are required — no hardcoded defaults.
#[derive(Debug, Clone, Builder, Deserialize)]
pub struct KnowledgeConfig {
    /// Whether the knowledge layer is active.
    pub enabled: bool,
    /// OpenAI embedding model name (e.g. "text-embedding-3-small").
    pub embedding_model: String,
    /// Embedding vector dimensions (e.g. 1536).
    pub embedding_dimensions: usize,
    /// Number of top-k results from usearch.
    pub search_top_k: usize,
    /// Cosine similarity threshold for deduplication (0.0–1.0).
    pub similarity_threshold: f32,
    /// LLM model name for memory extraction (e.g. "haiku").
    pub extractor_model: String,
}
```

**Step 2: Verify it compiles**

```bash
cargo check -p rara-kernel
```

**Step 3: Commit**

```bash
git add crates/kernel/src/memory/knowledge/config.rs
git commit -m "feat(kernel): add KnowledgeConfig struct (#N)"
```

---

## Task 4: Memory Items CRUD (`items.rs`)

**Files:**
- Create: `crates/kernel/src/memory/knowledge/items.rs`

This module handles all SQLite operations for `memory_items`.

**Step 1: Write the types and repository**

```rust
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

/// A single memory item stored in SQLite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryItem {
    pub id: i64,
    pub username: String,
    pub content: String,
    pub memory_type: String,
    pub category: String,
    pub source_tape: Option<String>,
    pub source_entry_id: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

/// Data needed to insert a new memory item (no id, no timestamps).
#[derive(Debug, Clone)]
pub struct NewMemoryItem {
    pub username: String,
    pub content: String,
    pub memory_type: String,
    pub category: String,
    pub source_tape: Option<String>,
    pub source_entry_id: Option<i64>,
    pub embedding: Option<Vec<u8>>,
}

/// Insert a new memory item. Returns the assigned row id.
pub async fn insert_item(pool: &SqlitePool, item: &NewMemoryItem) -> sqlx::Result<i64> {
    let result = sqlx::query_scalar!(
        r#"INSERT INTO memory_items (username, content, memory_type, category, source_tape, source_entry_id, embedding)
           VALUES (?, ?, ?, ?, ?, ?, ?)
           RETURNING id"#,
        item.username,
        item.content,
        item.memory_type,
        item.category,
        item.source_tape,
        item.source_entry_id,
        item.embedding,
    )
    .fetch_one(pool)
    .await?;
    Ok(result)
}

/// List all memory items for a user.
pub async fn list_items_by_username(pool: &SqlitePool, username: &str) -> sqlx::Result<Vec<MemoryItem>> {
    sqlx::query_as!(
        MemoryItem,
        r#"SELECT id, username, content, memory_type, category, source_tape, source_entry_id, created_at, updated_at
           FROM memory_items WHERE username = ? ORDER BY created_at DESC"#,
        username,
    )
    .fetch_all(pool)
    .await
}

/// List memory items for a user in a specific category.
pub async fn list_items_by_category(pool: &SqlitePool, username: &str, category: &str) -> sqlx::Result<Vec<MemoryItem>> {
    sqlx::query_as!(
        MemoryItem,
        r#"SELECT id, username, content, memory_type, category, source_tape, source_entry_id, created_at, updated_at
           FROM memory_items WHERE username = ? AND category = ? ORDER BY created_at DESC"#,
        username,
        category,
    )
    .fetch_all(pool)
    .await
}

/// Fetch items by a list of IDs (for usearch result lookup).
pub async fn get_items_by_ids(pool: &SqlitePool, ids: &[i64]) -> sqlx::Result<Vec<MemoryItem>> {
    // SQLite doesn't support array binds — build a comma-separated list.
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders: String = ids.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",");
    let query = format!(
        "SELECT id, username, content, memory_type, category, source_tape, source_entry_id, created_at, updated_at \
         FROM memory_items WHERE id IN ({placeholders}) ORDER BY created_at DESC"
    );
    sqlx::query_as::<_, MemoryItem>(&query)
        .fetch_all(pool)
        .await
}

/// Load the raw embedding blob for a specific item.
pub async fn get_embedding(pool: &SqlitePool, item_id: i64) -> sqlx::Result<Option<Vec<u8>>> {
    let row: Option<(Option<Vec<u8>>,)> = sqlx::query_as(
        "SELECT embedding FROM memory_items WHERE id = ?"
    )
    .bind(item_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|r| r.0))
}

/// Update the content of an existing item (for merge/dedup).
pub async fn update_item_content(pool: &SqlitePool, item_id: i64, content: &str) -> sqlx::Result<()> {
    sqlx::query("UPDATE memory_items SET content = ? WHERE id = ?")
        .bind(content)
        .bind(item_id)
        .execute(pool)
        .await?;
    Ok(())
}
```

**Step 2: Verify it compiles**

```bash
cargo check -p rara-kernel
```

**Step 3: Commit**

```bash
git add crates/kernel/src/memory/knowledge/items.rs
git commit -m "feat(kernel): memory items SQLite CRUD (#N)"
```

---

## Task 5: Category Files (`categories.rs`)

**Files:**
- Create: `crates/kernel/src/memory/knowledge/categories.rs`

Reads and writes category markdown files on disk.

**Step 1: Write the module**

```rust
use std::path::{Path, PathBuf};
use tokio::fs;

/// Metadata about a single category file.
#[derive(Debug, Clone)]
pub struct CategorySummary {
    pub name: String,
    pub preview: String,
}

/// Resolve the categories directory for a given username.
/// Path: `{data_dir}/memory/categories/{username}/`
fn categories_dir(username: &str) -> PathBuf {
    rara_paths::data_dir().join("memory/categories").join(username)
}

/// List all category files for a user, returning name + first few lines as preview.
pub async fn list_categories(username: &str) -> std::io::Result<Vec<CategorySummary>> {
    let dir = categories_dir(username);
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries = fs::read_dir(&dir).await?;
    let mut categories = Vec::new();

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let content = fs::read_to_string(&path).await.unwrap_or_default();
        let preview: String = content.lines().take(5).collect::<Vec<_>>().join("\n");
        categories.push(CategorySummary { name, preview });
    }

    Ok(categories)
}

/// Read the full content of a category file.
pub async fn read_category(username: &str, category: &str) -> std::io::Result<String> {
    let path = categories_dir(username).join(format!("{category}.md"));
    fs::read_to_string(&path).await
}

/// Write (create or overwrite) a category file.
pub async fn write_category(username: &str, category: &str, content: &str) -> std::io::Result<()> {
    let dir = categories_dir(username);
    fs::create_dir_all(&dir).await?;
    let path = dir.join(format!("{category}.md"));
    fs::write(&path, content).await
}

/// Check if a category file exists.
pub async fn category_exists(username: &str, category: &str) -> bool {
    let path = categories_dir(username).join(format!("{category}.md"));
    path.exists()
}
```

**Step 2: Verify it compiles**

```bash
cargo check -p rara-kernel
```

**Step 3: Commit**

```bash
git add crates/kernel/src/memory/knowledge/categories.rs
git commit -m "feat(kernel): category markdown file read/write (#N)"
```

---

## Task 6: Embedding Service (`embedding.rs`)

**Files:**
- Create: `crates/kernel/src/memory/knowledge/embedding.rs`

Handles OpenAI embedding API calls and usearch index management.

**Step 1: Write the embedding service**

```rust
use std::path::PathBuf;
use std::sync::Mutex;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

use super::config::KnowledgeConfig;

/// Manages embedding generation (OpenAI API) and vector search (usearch).
pub struct EmbeddingService {
    client: Client,
    config: KnowledgeConfig,
    index: Mutex<Index>,
    index_path: PathBuf,
}

#[derive(Serialize)]
struct EmbeddingRequest {
    model: String,
    input: Vec<String>,
    dimensions: usize,
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

impl EmbeddingService {
    /// Create a new EmbeddingService, loading or creating the usearch index.
    pub fn new(config: KnowledgeConfig, api_key: String) -> anyhow::Result<Self> {
        let index_path = rara_paths::data_dir().join("memory/memory.usearch");
        if let Some(parent) = index_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let options = IndexOptions {
            dimensions: config.embedding_dimensions,
            metric: MetricKind::Cos,
            quantization: ScalarKind::F32,
            ..Default::default()
        };
        let index = Index::new(&options)?;

        // Load existing index if present.
        if index_path.exists() {
            index.load(index_path.to_str().unwrap())?;
        }

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {api_key}").parse()?,
        );
        let client = Client::builder().default_headers(headers).build()?;

        Ok(Self {
            client,
            config,
            index: Mutex::new(index),
            index_path,
        })
    }

    /// Call OpenAI embedding API for a batch of texts.
    /// Returns one Vec<f32> per input text.
    pub async fn embed_texts(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let request = EmbeddingRequest {
            model: self.config.embedding_model.clone(),
            input: texts.to_vec(),
            dimensions: self.config.embedding_dimensions,
        };

        let response: EmbeddingResponse = self
            .client
            .post("https://api.openai.com/v1/embeddings")
            .json(&request)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        Ok(response.data.into_iter().map(|d| d.embedding).collect())
    }

    /// Add a vector to the usearch index. `key` is the memory_items row id.
    pub fn add_to_index(&self, key: u64, embedding: &[f32]) -> anyhow::Result<()> {
        let idx = self.index.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        idx.add(key, embedding)?;
        Ok(())
    }

    /// Search the usearch index for the top-k nearest neighbors.
    /// Returns Vec<(key, distance)> where key is memory_items.id.
    pub fn search(&self, query_embedding: &[f32], top_k: usize) -> anyhow::Result<Vec<(u64, f32)>> {
        let idx = self.index.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        let results = idx.search(query_embedding, top_k)?;
        Ok(results.keys.into_iter().zip(results.distances).collect())
    }

    /// Persist the usearch index to disk.
    pub fn save_index(&self) -> anyhow::Result<()> {
        let idx = self.index.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        idx.save(self.index_path.to_str().unwrap())?;
        Ok(())
    }

    /// Convert an f32 embedding Vec to a BLOB for SQLite storage.
    pub fn embedding_to_blob(embedding: &[f32]) -> Vec<u8> {
        embedding.iter().flat_map(|f| f.to_le_bytes()).collect()
    }

    /// Convert a SQLite BLOB back to Vec<f32>.
    pub fn blob_to_embedding(blob: &[u8]) -> Vec<f32> {
        blob.chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect()
    }

    pub fn similarity_threshold(&self) -> f32 {
        self.config.similarity_threshold
    }

    pub fn search_top_k(&self) -> usize {
        self.config.search_top_k
    }
}
```

**Step 2: Verify it compiles**

```bash
cargo check -p rara-kernel
```

**Step 3: Commit**

```bash
git add crates/kernel/src/memory/knowledge/embedding.rs
git commit -m "feat(kernel): embedding service with OpenAI API + usearch (#N)"
```

---

## Task 7: Memory Extractor (`extractor.rs`)

**Files:**
- Create: `crates/kernel/src/memory/knowledge/extractor.rs`

The core memorize pipeline: tape entries → LLM extraction → dedup → persist → update categories.

**Step 1: Write the extractor**

```rust
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tracing::{info, warn};

use super::{categories, embedding::EmbeddingService, items::{self, NewMemoryItem}};
use crate::llm::{CompletionRequest, LlmDriver, Message};
use crate::memory::{TapEntry, TapEntryKind};

/// A raw extracted memory from LLM output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedMemory {
    pub content: String,
    pub memory_type: String,
    pub category: String,
}

/// Run the full memorize pipeline for a completed session.
///
/// Steps:
/// 1. Filter tape entries to conversational content
/// 2. LLM extracts memory items
/// 3. Deduplicate against existing items via embedding similarity
/// 4. Persist new items to SQLite + usearch
/// 5. LLM updates category files
pub async fn extract_knowledge(
    entries: &[TapEntry],
    username: &str,
    tape_name: &str,
    pool: &SqlitePool,
    embedding_svc: &EmbeddingService,
    driver: &dyn LlmDriver,
    extractor_model: &str,
) -> anyhow::Result<usize> {
    // Step 1: Build conversation text from tape entries.
    let conversation = build_conversation_text(entries);
    if conversation.is_empty() {
        info!(username, "no conversation content to extract from");
        return Ok(0);
    }

    // Step 2: LLM extraction.
    let extracted = llm_extract_items(driver, extractor_model, &conversation).await?;
    if extracted.is_empty() {
        info!(username, "LLM extracted zero items");
        return Ok(0);
    }
    info!(username, count = extracted.len(), "LLM extracted memory items");

    // Step 3 + 4: Deduplicate and persist.
    let mut new_count = 0;
    let contents: Vec<String> = extracted.iter().map(|e| e.content.clone()).collect();
    let embeddings = embedding_svc.embed_texts(&contents).await?;

    for (item, emb) in extracted.iter().zip(embeddings.iter()) {
        // Check for duplicates.
        let similar = embedding_svc.search(emb, 1)?;
        if let Some((_, distance)) = similar.first() {
            // usearch cosine distance: 0.0 = identical, 2.0 = opposite.
            // Convert to similarity: 1.0 - distance/2.0
            let similarity = 1.0 - distance / 2.0;
            if similarity > embedding_svc.similarity_threshold() {
                continue; // Skip duplicate.
            }
        }

        let blob = EmbeddingService::embedding_to_blob(emb);
        let new_item = NewMemoryItem {
            username: username.to_string(),
            content: item.content.clone(),
            memory_type: item.memory_type.clone(),
            category: item.category.clone(),
            source_tape: Some(tape_name.to_string()),
            source_entry_id: None, // Could track first relevant entry id
            embedding: Some(blob),
        };

        let row_id = items::insert_item(pool, &new_item).await?;
        embedding_svc.add_to_index(row_id as u64, emb)?;
        new_count += 1;
    }

    // Save usearch index after batch insert.
    embedding_svc.save_index()?;

    if new_count == 0 {
        info!(username, "all extracted items were duplicates");
        return Ok(0);
    }

    // Step 5 + 6: Update category files.
    update_category_files(driver, extractor_model, username, pool).await?;

    info!(username, new_count, "knowledge extraction complete");
    Ok(new_count)
}

/// Build a plain-text conversation from tape entries for the extraction prompt.
fn build_conversation_text(entries: &[TapEntry]) -> String {
    let mut lines = Vec::new();
    for entry in entries {
        if entry.kind != TapEntryKind::Message {
            continue;
        }
        let role = entry.payload.get("role").and_then(|v| v.as_str()).unwrap_or("unknown");
        let content = entry
            .payload
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !content.is_empty() {
            lines.push(format!("[{role}]: {content}"));
        }
    }
    lines.join("\n")
}

/// Use LLM to extract memory items from conversation text.
async fn llm_extract_items(
    driver: &dyn LlmDriver,
    model: &str,
    conversation: &str,
) -> anyhow::Result<Vec<ExtractedMemory>> {
    let system_prompt = r#"You are a memory extraction agent. Given a conversation, extract key facts, preferences, events, habits, and skills about the user.

Output a JSON array where each element has:
- "content": a single natural language sentence describing the memory
- "memory_type": one of "preference", "fact", "event", "habit", "skill"
- "category": a short lowercase category name (e.g. "profile", "preferences", "work", "hobbies", "events")

Only extract information that is worth remembering long-term. Skip greetings, filler, and transient context.
Output ONLY the JSON array, no markdown fences or explanation."#;

    let request = CompletionRequest {
        model: model.to_string(),
        messages: vec![
            Message::system(system_prompt),
            Message::user(format!("Extract memories from this conversation:\n\n{conversation}")),
        ],
        ..Default::default()
    };

    let response = driver.complete(request).await?;
    let text = response.text().unwrap_or_default();

    // Parse JSON array from response.
    let items: Vec<ExtractedMemory> = serde_json::from_str(text.trim())
        .unwrap_or_else(|e| {
            warn!("failed to parse extraction output: {e}");
            Vec::new()
        });

    Ok(items)
}

/// Re-generate category files from current memory items.
async fn update_category_files(
    driver: &dyn LlmDriver,
    model: &str,
    username: &str,
    pool: &SqlitePool,
) -> anyhow::Result<()> {
    let all_items = items::list_items_by_username(pool, username).await?;
    if all_items.is_empty() {
        return Ok(());
    }

    // Group items by category.
    let mut by_category: std::collections::HashMap<String, Vec<&items::MemoryItem>> =
        std::collections::HashMap::new();
    for item in &all_items {
        by_category.entry(item.category.clone()).or_default().push(item);
    }

    for (category, cat_items) in &by_category {
        let existing = categories::read_category(username, category).await.unwrap_or_default();

        let items_text: String = cat_items
            .iter()
            .map(|i| format!("- [item:{}] [{}] {}", i.id, i.memory_type, i.content))
            .collect::<Vec<_>>()
            .join("\n");

        let prompt = format!(
            "You are a memory organizer. Update the following category file with these memory items.\n\n\
             Category: {category}\n\n\
             Current file content (may be empty):\n{existing}\n\n\
             Memory items:\n{items_text}\n\n\
             Write the updated markdown file. Organize items into logical sections. \
             Keep a ## 来源 section at the bottom listing all [item:N] references. \
             Output ONLY the markdown content, no fences."
        );

        let request = CompletionRequest {
            model: model.to_string(),
            messages: vec![
                Message::system("You are a structured knowledge organizer. Output clean markdown."),
                Message::user(prompt),
            ],
            ..Default::default()
        };

        let response = driver.complete(request).await?;
        let content = response.text().unwrap_or_default();
        if !content.is_empty() {
            categories::write_category(username, category, content).await?;
        }
    }

    Ok(())
}
```

**Note:** `CompletionRequest::default()` — check the actual struct. If it doesn't implement Default, construct it fully. The subagent should read `crates/kernel/src/llm/types.rs` to confirm the exact fields.

**Step 2: Verify it compiles**

```bash
cargo check -p rara-kernel
```

**Step 3: Commit**

```bash
git add crates/kernel/src/memory/knowledge/extractor.rs
git commit -m "feat(kernel): memory extraction pipeline with LLM (#N)"
```

---

## Task 8: Knowledge Module Root (`knowledge/mod.rs`)

**Files:**
- Create: `crates/kernel/src/memory/knowledge/mod.rs`
- Modify: `crates/kernel/src/memory/mod.rs` — add `pub mod knowledge;`

**Step 1: Write the knowledge module root**

```rust
pub mod categories;
pub mod config;
pub mod embedding;
pub mod extractor;
pub mod items;

pub use config::KnowledgeConfig;
pub use embedding::EmbeddingService;
```

**Step 2: Wire into memory module**

In `crates/kernel/src/memory/mod.rs`, add after the existing `mod` declarations:

```rust
pub mod knowledge;
```

**Step 3: Verify it compiles**

```bash
cargo check -p rara-kernel
```

**Step 4: Commit**

```bash
git add crates/kernel/src/memory/knowledge/mod.rs crates/kernel/src/memory/mod.rs
git commit -m "feat(kernel): wire knowledge module into memory (#N)"
```

---

## Task 9: MemoryTool — LLM-Callable Search Interface

**Files:**
- Create: `crates/kernel/src/memory/knowledge/tool.rs`

Implements `AgentTool` trait with three actions: `search`, `categories`, `read_category`.

**Step 1: Write MemoryTool**

```rust
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use sqlx::SqlitePool;

use super::{categories, embedding::EmbeddingService, items};
use crate::tool::{AgentTool, ToolContext};

/// LLM-callable tool for querying the Knowledge Layer.
pub struct MemoryTool {
    pool: SqlitePool,
    embedding_svc: Arc<EmbeddingService>,
}

impl MemoryTool {
    pub fn new(pool: SqlitePool, embedding_svc: Arc<EmbeddingService>) -> Self {
        Self { pool, embedding_svc }
    }
}

#[async_trait]
impl AgentTool for MemoryTool {
    fn name(&self) -> &str { "memory" }

    fn description(&self) -> &str {
        "Search and read the user's long-term memory. Supports three actions:\n\
         - search: semantic search across memory items\n\
         - categories: list all memory categories for the user\n\
         - read_category: read the full content of a specific category file"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["search", "categories", "read_category"],
                    "description": "The memory operation to perform"
                },
                "query": {
                    "type": "string",
                    "description": "Search query (required for 'search' action)"
                },
                "category": {
                    "type": "string",
                    "description": "Category name (required for 'read_category' action)"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: Value, context: &ToolContext) -> anyhow::Result<Value> {
        let action = params.get("action")
            .and_then(Value::as_str)
            .unwrap_or("");

        // Use user_id from ToolContext as username.
        let username = context.user_id.as_deref().unwrap_or("default");

        match action {
            "search" => {
                let query = params.get("query")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if query.is_empty() {
                    return Ok(json!({"error": "query is required for search action"}));
                }
                self.exec_search(username, query).await
            }
            "categories" => self.exec_categories(username).await,
            "read_category" => {
                let category = params.get("category")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if category.is_empty() {
                    return Ok(json!({"error": "category is required for read_category action"}));
                }
                self.exec_read_category(username, category).await
            }
            _ => Ok(json!({"error": format!("unknown action: {action}")})),
        }
    }
}

impl MemoryTool {
    async fn exec_search(&self, username: &str, query: &str) -> anyhow::Result<Value> {
        // Embed the query.
        let embeddings = self.embedding_svc.embed_texts(&[query.to_string()]).await?;
        let query_emb = embeddings.first()
            .ok_or_else(|| anyhow::anyhow!("empty embedding response"))?;

        // Search usearch index.
        let top_k = self.embedding_svc.search_top_k();
        let results = self.embedding_svc.search(query_emb, top_k)?;

        // Fetch matching items from SQLite.
        let ids: Vec<i64> = results.iter().map(|(key, _)| *key as i64).collect();
        let mut matched_items = items::get_items_by_ids(&self.pool, &ids).await?;

        // Filter by username.
        matched_items.retain(|item| item.username == username);

        let items_json: Vec<Value> = matched_items.iter().map(|item| {
            json!({
                "id": item.id,
                "content": item.content,
                "memory_type": item.memory_type,
                "category": item.category,
            })
        }).collect();

        Ok(json!({"items": items_json}))
    }

    async fn exec_categories(&self, username: &str) -> anyhow::Result<Value> {
        let cats = categories::list_categories(username).await?;
        let cats_json: Vec<Value> = cats.iter().map(|c| {
            json!({
                "name": c.name,
                "preview": c.preview,
            })
        }).collect();
        Ok(json!({"categories": cats_json}))
    }

    async fn exec_read_category(&self, username: &str, category: &str) -> anyhow::Result<Value> {
        match categories::read_category(username, category).await {
            Ok(content) => Ok(json!({"category": category, "content": content})),
            Err(_) => Ok(json!({"error": format!("category '{category}' not found")})),
        }
    }
}
```

**Step 2: Add `pub mod tool;` to `knowledge/mod.rs`**

**Step 3: Verify it compiles**

```bash
cargo check -p rara-kernel
```

**Step 4: Commit**

```bash
git add crates/kernel/src/memory/knowledge/tool.rs crates/kernel/src/memory/knowledge/mod.rs
git commit -m "feat(kernel): MemoryTool with search/categories/read_category (#N)"
```

---

## Task 10: Wire into Kernel — Event Handling + Tool Registration

**Files:**
- Modify: `crates/kernel/src/event.rs` — (potentially) no changes needed, use existing `SessionCommand` / `TurnCompleted`
- Modify: `crates/kernel/src/kernel.rs` — add extraction trigger after turn completion

This task wires the knowledge layer into the kernel event loop. The extraction should happen after a session's final turn completes.

**Step 1: Understand the wiring point**

In `kernel.rs`, `handle_turn_completed()` runs after each LLM turn finishes. When a session ends (transitions to terminal state), that's where we spawn the async extraction task.

The subagent should read:
- `crates/kernel/src/kernel.rs` — `handle_turn_completed()` method (around line 1494)
- How `Kernel` holds `tape_service` and can access the DB pool
- How `LlmDriver` is accessed via `SyscallDispatcher.driver_registry`

**Step 2: Add KnowledgeStore to Kernel**

The `Kernel` struct needs access to `Arc<EmbeddingService>` and `SqlitePool`. These should be passed through `Kernel::new()` or constructed during boot. The subagent should:

1. Add `knowledge: Option<Arc<KnowledgeService>>` field to `Kernel` (where `KnowledgeService` bundles pool + embedding_svc + config)
2. After `handle_turn_completed` transitions a session to terminal state, if knowledge is enabled, spawn `tokio::spawn` calling `extractor::extract_knowledge()`
3. Register `MemoryTool` in the tool registry during boot

**Step 3: Extend context injection**

In `context.rs`, add a new function `knowledge_context()` that loads category summaries and returns an optional system message, similar to `user_tape_context()`. Wire it into the context building path.

**Important:** The subagent should read the actual current code of `kernel.rs`, `syscall.rs`, and the boot/app crate to understand exactly how to wire this in. The code snippets above are guidance, not copy-paste ready — the subagent must adapt to the actual codebase state.

**Step 4: Verify it compiles**

```bash
cargo check -p rara-kernel
```

**Step 5: Commit**

```bash
git add crates/kernel/src/kernel.rs crates/kernel/src/memory/context.rs
git commit -m "feat(kernel): wire knowledge extraction into event loop (#N)"
```

---

## Task 11: Update Config Loading in App

**Files:**
- Modify: wherever config is loaded (check `crates/app/` or config loading code)
- Modify: `config.example.yaml` — add `memory.knowledge` section

**Step 1: Add KnowledgeConfig to the app config struct**

The subagent should find where the top-level config struct is defined (likely in app crate or a shared config module) and add:

```yaml
memory:
  knowledge:
    enabled: true
    embedding_model: "text-embedding-3-small"
    embedding_dimensions: 1536
    search_top_k: 20
    similarity_threshold: 0.85
    extractor_model: "haiku"
```

**Step 2: Pass config to Kernel**

Thread `KnowledgeConfig` through the boot sequence into `Kernel::new()`.

**Step 3: Verify it compiles**

```bash
cargo check -p rara-kernel
```

**Step 4: Commit**

```bash
git add -A
git commit -m "feat(app): wire knowledge config into boot sequence (#N)"
```

---

## Task Dependencies

```
Task 1 (migration)     ─┐
Task 2 (usearch dep)   ─┤
Task 3 (config)        ─┼─► Task 8 (mod.rs) ─► Task 10 (kernel wiring)
Task 4 (items.rs)      ─┤                     ─► Task 11 (config loading)
Task 5 (categories.rs) ─┤
Task 6 (embedding.rs)  ─┤
Task 7 (extractor.rs)  ─┘
Task 9 (tool.rs)       ─────► Task 10

Parallelizable: Tasks 1-7 can all be done in parallel.
Task 8 depends on 3-7.
Task 9 depends on 4-6.
Task 10 depends on 8, 9.
Task 11 depends on 3, 10.
```
