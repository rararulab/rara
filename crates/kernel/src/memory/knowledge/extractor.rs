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

//! Memory extraction pipeline — tape entries -> LLM extraction -> dedup ->
//! persist -> category update.
//!
//! This module is the core of the knowledge layer's write path. After a session
//! completes, [`extract_knowledge`] processes the conversation tape to extract
//! long-term memory items, deduplicates them against existing knowledge via
//! embedding similarity, persists new items, and regenerates category files.

use serde::{Deserialize, Serialize};
use snafu::prelude::*;
use sqlx::SqlitePool;
use tracing::{info, warn};

use super::{
    categories,
    embedding::{self, EmbeddingService},
    items::{self, NewMemoryItem},
};
use crate::{
    llm::{CompletionRequest, LlmDriver, Message, ToolChoice},
    memory::{TapEntry, TapEntryKind},
};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Typed errors for the knowledge extraction pipeline.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum ExtractorError {
    /// LLM completion call failed.
    #[snafu(display("LLM call failed: {source}"))]
    Llm { source: crate::error::KernelError },

    /// Embedding operation failed.
    #[snafu(display("embedding failed: {source}"))]
    Embedding {
        source: super::embedding::EmbeddingError,
    },

    /// Database operation failed.
    #[snafu(display("database error: {source}"))]
    Database { source: sqlx::Error },

    /// Category file I/O failed.
    #[snafu(display("category error: {source}"))]
    Category {
        source: super::categories::CategoryError,
    },
}

/// Result alias for [`ExtractorError`].
pub type Result<T> = std::result::Result<T, ExtractorError>;

/// A raw extracted memory from LLM output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedMemory {
    pub content:     String,
    pub memory_type: String,
    pub category:    String,
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
    similarity_threshold: f32,
) -> Result<usize> {
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
    info!(
        username,
        count = extracted.len(),
        "LLM extracted memory items"
    );

    // Step 3 + 4: Deduplicate and persist.
    let mut new_count = 0;
    let contents: Vec<String> = extracted.iter().map(|e| e.content.clone()).collect();
    let embeddings = embedding_svc
        .embed(&contents)
        .await
        .context(EmbeddingSnafu)?;

    for (item, emb) in extracted.iter().zip(embeddings.iter()) {
        // Check for duplicates via vector similarity.
        let similar = embedding_svc.search(emb, 1).context(EmbeddingSnafu)?;
        if let Some(&(_, distance)) = similar.first() {
            // usearch cosine distance: 0.0 = identical, 2.0 = opposite.
            // Convert to similarity: 1.0 - distance/2.0
            let similarity = 1.0 - distance / 2.0;
            if similarity > similarity_threshold {
                continue; // Skip duplicate.
            }
        }

        let blob = embedding::embedding_to_blob(emb);
        let new_item = NewMemoryItem {
            username:        username.to_string(),
            content:         item.content.clone(),
            memory_type:     item.memory_type.clone(),
            category:        item.category.clone(),
            source_tape:     Some(tape_name.to_string()),
            source_entry_id: None,
            embedding:       Some(blob),
        };

        let row_id = items::insert_item(pool, &new_item)
            .await
            .context(DatabaseSnafu)?;
        embedding_svc
            .add_to_index(row_id as u64, emb)
            .context(EmbeddingSnafu)?;
        new_count += 1;
    }

    // Save usearch index after batch insert.
    if new_count > 0 {
        embedding_svc.save_index().context(EmbeddingSnafu)?;
    }

    if new_count == 0 {
        info!(username, "all extracted items were duplicates");
        return Ok(0);
    }

    // Step 5: Update category files.
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
        let role = entry
            .payload
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
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
) -> Result<Vec<ExtractedMemory>> {
    let system_prompt = r#"You are a memory extraction agent. Given a conversation, extract key facts, preferences, events, habits, and skills about the user.

Output a JSON array where each element has:
- "content": a single natural language sentence describing the memory
- "memory_type": one of "preference", "fact", "event", "habit", "skill"
- "category": a short lowercase category name (e.g. "profile", "preferences", "work", "hobbies", "events")

Only extract facts that would change how you interact with this user in a future conversation. Skip greetings, filler, and transient context.
Output ONLY the JSON array, no markdown fences or explanation."#;

    let request = CompletionRequest {
        model:               model.to_string(),
        messages:            vec![
            Message::system(system_prompt),
            Message::user(format!(
                "Extract memories from this conversation:\n\n{conversation}"
            )),
        ],
        tools:               Vec::new(),
        temperature:         Some(0.2),
        max_tokens:          Some(4096),
        thinking:            None,
        tool_choice:         ToolChoice::None,
        parallel_tool_calls: false,
        frequency_penalty:   None,
        top_p:               None,
    };

    let response = driver.complete(request).await.context(LlmSnafu)?;
    let text = response.content.unwrap_or_default();

    // Parse JSON array from response.
    let items: Vec<ExtractedMemory> = serde_json::from_str(text.trim()).unwrap_or_else(|e| {
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
) -> Result<()> {
    let all_items = items::list_items_by_username(pool, username)
        .await
        .context(DatabaseSnafu)?;
    if all_items.is_empty() {
        return Ok(());
    }

    // Group items by category.
    let mut by_category: std::collections::HashMap<String, Vec<&items::MemoryItem>> =
        std::collections::HashMap::new();
    for item in &all_items {
        by_category
            .entry(item.category.clone())
            .or_default()
            .push(item);
    }

    for (category, cat_items) in &by_category {
        let existing = categories::read_category(username, category)
            .await
            .context(CategorySnafu)?
            .unwrap_or_default();

        let items_text: String = cat_items
            .iter()
            .map(|i| format!("- [item:{}] [{}] {}", i.id, i.memory_type, i.content))
            .collect::<Vec<_>>()
            .join("\n");

        let prompt = format!(
            "You are a memory organizer. Update the following category file with these memory \
             items.\n\nCategory: {category}\n\nCurrent file content (may be \
             empty):\n{existing}\n\nMemory items:\n{items_text}\n\nWrite the updated markdown \
             file. Organize items into logical sections. Output ONLY the markdown content, no \
             fences."
        );

        let request = CompletionRequest {
            model:               model.to_string(),
            messages:            vec![
                Message::system("You are a structured knowledge organizer. Output clean markdown."),
                Message::user(prompt),
            ],
            tools:               Vec::new(),
            temperature:         Some(0.3),
            max_tokens:          Some(4096),
            thinking:            None,
            tool_choice:         ToolChoice::None,
            parallel_tool_calls: false,
            frequency_penalty:   None,
            top_p:               None,
        };

        let response = driver.complete(request).await.context(LlmSnafu)?;
        let content = response.content.unwrap_or_default();
        if !content.is_empty() {
            categories::write_category(username, category, &content)
                .await
                .context(CategorySnafu)?;
        }
    }

    Ok(())
}
