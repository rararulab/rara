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

//! Category markdown files — Layer 3 of the knowledge architecture.
//!
//! Each `(username, category)` pair maps to a markdown file on disk at
//! `<knowledge_dir>/<username>/<category>.md`. These files are LLM-generated
//! summaries that condense raw memory items into a coherent narrative.

use std::path::PathBuf;

use tokio::fs;

/// Return the base directory for a user's knowledge categories.
fn user_knowledge_dir(username: &str) -> PathBuf {
    rara_paths::data_dir().join("knowledge").join(username)
}

/// Path to a specific category file.
fn category_path(username: &str, category: &str) -> PathBuf {
    user_knowledge_dir(username).join(format!("{category}.md"))
}

/// List all category names for a user by scanning the knowledge directory.
pub async fn list_categories(username: &str) -> anyhow::Result<Vec<String>> {
    let dir = user_knowledge_dir(username);
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries = fs::read_dir(&dir).await?;
    let mut categories = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                categories.push(stem.to_owned());
            }
        }
    }
    categories.sort();
    Ok(categories)
}

/// Read the content of a category markdown file.
pub async fn read_category(username: &str, category: &str) -> anyhow::Result<Option<String>> {
    let path = category_path(username, category);
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path).await?;
    Ok(Some(content))
}

/// Write (overwrite) a category markdown file.
pub async fn write_category(
    username: &str,
    category: &str,
    content: &str,
) -> anyhow::Result<()> {
    let dir = user_knowledge_dir(username);
    fs::create_dir_all(&dir).await?;
    let path = category_path(username, category);
    fs::write(&path, content).await?;
    Ok(())
}
