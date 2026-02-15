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

//! Layer 2 service tools for memory retrieval and writing.

use std::sync::Arc;

use async_trait::async_trait;
use rara_agents::tool_registry::AgentTool;
use rara_memory::MemoryManager;
use serde_json::json;

/// Search local memory index (keyword/hybrid depending on runtime settings).
///
/// Sync is handled by the background `MemorySyncWorker`; this tool only
/// queries the already-indexed data.
pub struct MemorySearchTool {
    manager: Arc<MemoryManager>,
}

impl MemorySearchTool {
    /// Create a `memory_search` tool.
    pub fn new(manager: Arc<MemoryManager>) -> Self { Self { manager } }
}

#[async_trait]
impl AgentTool for MemorySearchTool {
    fn name(&self) -> &str { "memory_search" }

    fn description(&self) -> &str {
        "Search long-term memory documents (Markdown index). Returns relevant chunk IDs and \
         snippets."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Keyword query for searching memory"
                },
                "limit": {
                    "type": "number",
                    "description": "Maximum number of results (default 8, max 50)"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
    ) -> rara_agents::err::Result<serde_json::Value> {
        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| rara_agents::err::Error::Other {
                message: "missing required parameter: query".into(),
            })?;

        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .map_or(8_usize, |v| v as usize)
            .clamp(1, 50);

        let results = self.manager.search(query, limit).await.map_err(|e| {
            rara_agents::err::Error::Other {
                message: format!("memory search failed: {e}").into(),
            }
        })?;

        Ok(json!({
            "query": query,
            "count": results.len(),
            "results": results
                .iter()
                .map(|r| json!({
                    "chunk_id": r.chunk_id,
                    "path": r.path,
                    "chunk_index": r.chunk_index,
                    "score": r.score,
                    "snippet": r.snippet,
                }))
                .collect::<Vec<_>>()
        }))
    }
}

/// Retrieve full chunk content by chunk ID.
pub struct MemoryGetTool {
    manager: Arc<MemoryManager>,
}

impl MemoryGetTool {
    /// Create a `memory_get` tool.
    pub fn new(manager: Arc<MemoryManager>) -> Self { Self { manager } }
}

#[async_trait]
impl AgentTool for MemoryGetTool {
    fn name(&self) -> &str { "memory_get" }

    fn description(&self) -> &str {
        "Get full memory chunk content by chunk_id from local memory index."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "chunk_id": {
                    "type": "number",
                    "description": "Chunk ID returned by memory_search"
                }
            },
            "required": ["chunk_id"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
    ) -> rara_agents::err::Result<serde_json::Value> {
        let chunk_id = params
            .get("chunk_id")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| rara_agents::err::Error::Other {
                message: "missing required parameter: chunk_id".into(),
            })?;

        match self.manager.get_chunk(chunk_id).await.map_err(|e| {
            rara_agents::err::Error::Other {
                message: format!("memory get failed: {e}").into(),
            }
        })? {
            Some(chunk) => Ok(json!({
                "chunk_id": chunk.chunk_id,
                "path": chunk.path,
                "chunk_index": chunk.chunk_index,
                "content": chunk.content,
            })),
            None => Ok(json!({
                "error": format!("chunk not found: {chunk_id}")
            })),
        }
    }
}

/// Update a specific section of the persistent user profile.
///
/// The profile is stored as `user_profile.md` in the memory directory.
/// Each section is a level-2 heading (`## Section Name`) and this tool
/// replaces the content between the targeted heading and the next heading
/// (or end-of-file).
pub struct MemoryUpdateProfileTool {
    manager: Arc<MemoryManager>,
}

impl MemoryUpdateProfileTool {
    /// Create a `memory_update_profile` tool.
    pub fn new(manager: Arc<MemoryManager>) -> Self { Self { manager } }
}

/// Default template created when no profile exists yet.
const PROFILE_TEMPLATE: &str = "\
# User Profile

## Basic Info

## Preferences

## Current Goals

## Key Context
";

/// Replace the content of `## {section}` with `new_content`, preserving
/// everything else. If the section doesn't exist, append it.
fn replace_section(profile: &str, section: &str, new_content: &str) -> String {
    let header = format!("## {section}");
    let lines: Vec<&str> = profile.lines().collect();

    // Find the line index of the target header.
    let header_idx = lines.iter().position(|line| line.trim() == header);

    match header_idx {
        Some(idx) => {
            // Find the next `##` header after idx (or end of file).
            let next_header_idx = lines
                .iter()
                .enumerate()
                .skip(idx + 1)
                .find(|(_, line)| line.starts_with("## "))
                .map(|(i, _)| i)
                .unwrap_or(lines.len());

            // Build the replacement slice: header + blank + content + blank
            let mut replacement: Vec<String> = vec![header.clone(), String::new()];
            for line in new_content.lines() {
                replacement.push(line.to_owned());
            }
            replacement.push(String::new());

            // Replace lines[idx..next_header_idx] with replacement.
            let mut result: Vec<String> = Vec::new();
            for line in &lines[..idx] {
                result.push((*line).to_owned());
            }
            result.extend(replacement);
            for line in &lines[next_header_idx..] {
                result.push((*line).to_owned());
            }
            result.join("\n")
        }
        None => {
            // Section not found — append it.
            let mut result = profile.to_owned();
            if !result.ends_with('\n') {
                result.push('\n');
            }
            result.push_str(&format!("\n{header}\n\n{new_content}\n"));
            result
        }
    }
}

#[async_trait]
impl AgentTool for MemoryUpdateProfileTool {
    fn name(&self) -> &str { "memory_update_profile" }

    fn description(&self) -> &str {
        "Update a specific section of the persistent user profile. Sections: Basic Info, \
         Preferences, Current Goals, Key Context."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "section": {
                    "type": "string",
                    "description": "Section header to update (e.g. 'Basic Info', 'Preferences', 'Current Goals', 'Key Context')"
                },
                "content": {
                    "type": "string",
                    "description": "New content for that section"
                }
            },
            "required": ["section", "content"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
    ) -> rara_agents::err::Result<serde_json::Value> {
        let section = params
            .get("section")
            .and_then(|v| v.as_str())
            .ok_or_else(|| rara_agents::err::Error::Other {
                message: "missing required parameter: section".into(),
            })?;

        let content = params
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| rara_agents::err::Error::Other {
                message: "missing required parameter: content".into(),
            })?;

        // Read current profile (empty string if not yet created).
        let mut profile = self.manager.read_core_profile().await.map_err(|e| {
            rara_agents::err::Error::Other {
                message: format!("failed to read profile: {e}").into(),
            }
        })?;

        // Initialize with template if empty.
        if profile.trim().is_empty() {
            profile = PROFILE_TEMPLATE.to_owned();
        }

        // Replace the target section.
        let updated = replace_section(&profile, section, content);

        // Write back.
        self.manager
            .write_core_profile(&updated)
            .await
            .map_err(|e| rara_agents::err::Error::Other {
                message: format!("failed to write profile: {e}").into(),
            })?;

        // Trigger sync so the profile is indexed for search.
        self.manager
            .sync()
            .await
            .map_err(|e| rara_agents::err::Error::Other {
                message: format!("memory sync failed: {e}").into(),
            })?;

        Ok(json!({
            "status": "ok",
            "section": section,
            "message": format!("Profile section '{}' updated", section),
        }))
    }
}

/// Write markdown content to the memory directory and trigger a sync.
///
/// This allows agents to persist notes, summaries, or any markdown document
/// into long-term memory so it becomes searchable via `memory_search`.
pub struct MemoryWriteTool {
    manager: Arc<MemoryManager>,
}

impl MemoryWriteTool {
    /// Create a `memory_write` tool.
    pub fn new(manager: Arc<MemoryManager>) -> Self { Self { manager } }
}

#[async_trait]
impl AgentTool for MemoryWriteTool {
    fn name(&self) -> &str { "memory_write" }

    fn description(&self) -> &str {
        "Write markdown content to long-term memory. The file will be indexed and searchable via \
         memory_search."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "filename": {
                    "type": "string",
                    "description": "Filename for the memory document (e.g. 'meeting-notes.md'). Auto-generated if omitted."
                },
                "content": {
                    "type": "string",
                    "description": "Markdown content to write to memory"
                }
            },
            "required": ["content"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
    ) -> rara_agents::err::Result<serde_json::Value> {
        let content = params
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| rara_agents::err::Error::Other {
                message: "missing required parameter: content".into(),
            })?;

        let filename = match params.get("filename").and_then(|v| v.as_str()) {
            Some(name) => {
                // Ensure .md extension
                if name.ends_with(".md") {
                    name.to_owned()
                } else {
                    format!("{name}.md")
                }
            }
            None => {
                let ts = jiff::Timestamp::now().as_second();
                format!("agent-{ts}.md")
            }
        };

        let memory_dir = rara_paths::memory_dir();
        let file_path = memory_dir.join(&filename);

        // Ensure parent directory exists (in case filename contains subdirectories).
        if let Some(parent) = file_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                rara_agents::err::Error::Other {
                    message: format!("failed to create directory: {e}").into(),
                }
            })?;
        }

        tokio::fs::write(&file_path, content).await.map_err(|e| {
            rara_agents::err::Error::Other {
                message: format!("failed to write memory file: {e}").into(),
            }
        })?;

        // Trigger sync so the new file is immediately indexed.
        self.manager
            .sync()
            .await
            .map_err(|e| rara_agents::err::Error::Other {
                message: format!("memory sync failed: {e}").into(),
            })?;

        Ok(json!({
            "status": "ok",
            "filename": filename,
            "path": file_path.to_string_lossy(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_existing_section() {
        let profile = "\
# User Profile

## Basic Info

- Name: Alice

## Preferences

- Language: English

## Current Goals
";
        let result = replace_section(profile, "Basic Info", "- Name: Bob\n- Role: Engineer");
        assert!(result.contains("- Name: Bob"));
        assert!(result.contains("- Role: Engineer"));
        assert!(!result.contains("- Name: Alice"));
        // Other sections should be preserved.
        assert!(result.contains("## Preferences"));
        assert!(result.contains("- Language: English"));
    }

    #[test]
    fn replace_last_section() {
        let profile = "\
# User Profile

## Basic Info

- Name: Alice

## Current Goals

- Find a job";
        let result = replace_section(profile, "Current Goals", "- Learn Rust");
        assert!(result.contains("- Learn Rust"));
        assert!(!result.contains("- Find a job"));
        assert!(result.contains("- Name: Alice"));
    }

    #[test]
    fn append_missing_section() {
        let profile = "\
# User Profile

## Basic Info

- Name: Alice
";
        let result = replace_section(profile, "Key Context", "- Likes cats");
        assert!(result.contains("## Key Context"));
        assert!(result.contains("- Likes cats"));
        assert!(result.contains("- Name: Alice"));
    }

    #[test]
    fn replace_section_in_template() {
        let result = replace_section(PROFILE_TEMPLATE, "Basic Info", "- Name: Ryan");
        assert!(result.contains("- Name: Ryan"));
        assert!(result.contains("## Preferences"));
        assert!(result.contains("## Current Goals"));
        assert!(result.contains("## Key Context"));
    }
}
