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

use std::collections::HashMap;

use super::{
    PromptRepo,
    types::{PromptEntry, PromptSpec},
};

/// Read-only prompt repository backed by compiled-in defaults.
///
/// The map is built once at construction time and never mutated,
/// so no locking is needed.
pub struct BuiltinPromptRepo {
    entries: HashMap<String, PromptEntry>,
}

impl BuiltinPromptRepo {
    /// Build a new repository from the given prompt specifications.
    #[must_use]
    pub fn new(specs: Vec<PromptSpec>) -> Self {
        let mut entries = HashMap::with_capacity(specs.len());
        for spec in specs {
            entries.insert(
                spec.name.to_owned(),
                PromptEntry {
                    name:        spec.name.to_owned(),
                    description: spec.description.to_owned(),
                    content:     spec.default_content.to_owned(),
                },
            );
        }
        Self { entries }
    }
}

#[async_trait::async_trait]
impl PromptRepo for BuiltinPromptRepo {
    async fn get(&self, name: &str) -> Option<PromptEntry> { self.entries.get(name).cloned() }

    async fn list(&self) -> Vec<PromptEntry> { self.entries.values().cloned().collect() }
}

/// Returns all built-in prompt specifications with their compiled-in defaults.
///
/// This is the **single source of truth** for prompt registration. Every
/// `include_str!()` referencing prompts should live here and nowhere else.
#[must_use]
pub fn all_builtin_prompts() -> Vec<PromptSpec> {
    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_specs() -> Vec<PromptSpec> {
        vec![
            PromptSpec {
                name:            "test/hello.md",
                description:     "Test prompt",
                default_content: "Hello, world!",
            },
            PromptSpec {
                name:            "test/nested/deep.md",
                description:     "Nested prompt",
                default_content: "Deep content",
            },
        ]
    }

    #[tokio::test]
    async fn get_returns_builtin_content() {
        let repo = BuiltinPromptRepo::new(test_specs());

        let entry = repo.get("test/hello.md").await.unwrap();
        assert_eq!(entry.content, "Hello, world!");
        assert_eq!(entry.description, "Test prompt");
    }

    #[tokio::test]
    async fn get_returns_none_for_unknown() {
        let repo = BuiltinPromptRepo::new(test_specs());
        assert!(repo.get("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn list_returns_all_entries() {
        let repo = BuiltinPromptRepo::new(test_specs());
        let entries = repo.list().await;
        assert_eq!(entries.len(), 2);
    }
}
