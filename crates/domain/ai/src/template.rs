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

//! Prompt template management and rendering.
//!
//! Templates use a simple `{{variable}}` substitution syntax.
//! The [`PromptTemplateManager`] trait abstracts over how templates are
//! loaded (from a database, from files, etc.) while
//! [`render`] performs the actual variable replacement.

use std::collections::HashMap;

use crate::{error::AiError, kind::AiTaskKind};

/// Minimal template descriptor used within the domain layer.
///
/// This is intentionally decoupled from the store-layer
/// `PromptTemplate` so that the domain crate does not depend on
/// infrastructure.
#[derive(Debug, Clone)]
pub struct PromptTemplate {
    /// Unique name / slug for the template.
    pub name:    String,
    /// The template content with `{{variable}}` placeholders.
    pub content: String,
    /// Version number (for auditing / rollback).
    pub version: i32,
}

/// Trait for loading prompt templates.
///
/// Implementations may read from a database, from disk, or from an
/// in-memory registry.
#[async_trait::async_trait]
pub trait PromptTemplateManager: Send + Sync {
    /// Load a template by its unique name.
    async fn get_by_name(&self, name: &str) -> Result<Option<PromptTemplate>, AiError>;

    /// Load the active template for a given task kind.
    ///
    /// The implementation is expected to select the latest active
    /// version for the kind.
    async fn get_for_task_kind(&self, kind: AiTaskKind) -> Result<Option<PromptTemplate>, AiError>;
}

/// Render a template by replacing `{{key}}` placeholders with values
/// from `vars`.
///
/// # Errors
///
/// Returns [`AiError::MissingTemplateVariable`] if the template
/// references a variable that is not present in `vars`.
pub fn render<S: ::std::hash::BuildHasher>(
    template: &str,
    vars: &HashMap<String, String, S>,
) -> Result<String, AiError> {
    let mut result = template.to_owned();
    let mut start = 0;

    // Scan for `{{...}}` patterns and replace them.
    while let Some(open) = result[start..].find("{{") {
        let open = start + open;
        let Some(close) = result[open..].find("}}") else {
            // Unterminated `{{` -- treat as literal text.
            break;
        };
        let close = open + close;

        let var_name = result[open + 2..close].trim();

        let value = vars
            .get(var_name)
            .ok_or_else(|| AiError::MissingTemplateVariable {
                variable: var_name.to_owned(),
            })?;

        result.replace_range(open..close + 2, value);

        // Continue scanning after the replaced value.
        start = open + value.len();
    }

    Ok(result)
}

/// An in-memory template manager backed by a simple `HashMap`.
///
/// Useful for testing and for providing built-in default templates
/// that do not require a database.
#[derive(Debug, Clone, Default)]
pub struct InMemoryTemplateManager {
    templates: HashMap<String, PromptTemplate>,
}

impl InMemoryTemplateManager {
    /// Create a new empty manager.
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Register a template. Overwrites any existing template with the
    /// same name.
    pub fn insert(&mut self, template: PromptTemplate) {
        self.templates.insert(template.name.clone(), template);
    }
}

#[async_trait::async_trait]
impl PromptTemplateManager for InMemoryTemplateManager {
    async fn get_by_name(&self, name: &str) -> Result<Option<PromptTemplate>, AiError> {
        Ok(self.templates.get(name).cloned())
    }

    async fn get_for_task_kind(&self, kind: AiTaskKind) -> Result<Option<PromptTemplate>, AiError> {
        // Convention: the template name for a task kind is its
        // snake_case string representation.
        let key = kind.to_string();
        Ok(self.templates.get(&key).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_replaces_variables() {
        let tpl = "Hello, {{name}}! Your role is {{role}}.";
        let mut vars = HashMap::new();
        vars.insert("name".to_owned(), "Alice".to_owned());
        vars.insert("role".to_owned(), "Engineer".to_owned());

        let result = render(tpl, &vars).unwrap();
        assert_eq!(result, "Hello, Alice! Your role is Engineer.");
    }

    #[test]
    fn render_returns_error_on_missing_variable() {
        let tpl = "Dear {{name}}, welcome to {{company}}.";
        let mut vars = HashMap::new();
        vars.insert("name".to_owned(), "Bob".to_owned());

        let err = render(tpl, &vars).unwrap_err();
        assert!(err.to_string().contains("company"));
    }

    #[test]
    fn render_handles_no_placeholders() {
        let tpl = "No placeholders here.";
        let vars = HashMap::new();
        let result = render(tpl, &vars).unwrap();
        assert_eq!(result, "No placeholders here.");
    }

    #[test]
    fn render_handles_adjacent_placeholders() {
        let tpl = "{{a}}{{b}}";
        let mut vars = HashMap::new();
        vars.insert("a".to_owned(), "X".to_owned());
        vars.insert("b".to_owned(), "Y".to_owned());

        let result = render(tpl, &vars).unwrap();
        assert_eq!(result, "XY");
    }
}
