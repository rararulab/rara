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

//! Just command runner for Typst projects.
//!
//! Provides the ability to list and run `just` recipes in a project directory,
//! as well as run arbitrary shell commands.

use std::path::Path;

use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::error::TypstError;

/// A justfile recipe description.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct JustRecipe {
    pub name:        String,
    pub description: Option<String>,
}

/// Command execution result.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RunOutput {
    pub exit_code: i32,
    pub stdout:    String,
    pub stderr:    String,
}

/// Check whether a project directory contains a justfile.
pub fn has_justfile(project_dir: &Path) -> bool {
    project_dir.join("justfile").exists()
        || project_dir.join("Justfile").exists()
        || project_dir.join(".justfile").exists()
}

/// Parse the justfile in a project directory and return available recipes.
///
/// Runs `just --list --unsorted` and parses the output. The typical format is:
///
/// ```text
/// Available recipes:
///     recipe-name # description
///     another
/// ```
pub async fn list_recipes(project_dir: &Path) -> Result<Vec<JustRecipe>, TypstError> {
    let output = Command::new("just")
        .args(["--list", "--unsorted"])
        .current_dir(project_dir)
        .output()
        .await
        .map_err(|e| TypstError::CommandFailed {
            message: format!("failed to run just: {e}"),
        })?;

    if !output.status.success() {
        return Err(TypstError::CommandFailed {
            message: format!(
                "just --list failed (exit {}): {}",
                output.status.code().unwrap_or(-1),
                String::from_utf8_lossy(&output.stderr)
            ),
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let recipes = stdout
        .lines()
        // Skip the header line ("Available recipes:")
        .skip(1)
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }
            // Format: "recipe-name # description" or just "recipe-name"
            if let Some((name_part, desc_part)) = trimmed.split_once('#') {
                let name = name_part.trim().to_owned();
                let desc = desc_part.trim().to_owned();
                if name.is_empty() {
                    return None;
                }
                Some(JustRecipe {
                    name,
                    description: if desc.is_empty() { None } else { Some(desc) },
                })
            } else {
                let name = trimmed.to_owned();
                if name.is_empty() {
                    return None;
                }
                Some(JustRecipe {
                    name,
                    description: None,
                })
            }
        })
        .collect();

    Ok(recipes)
}

/// Run a specific just recipe in the project directory.
///
/// Validates that the recipe name contains only safe characters (alphanumeric,
/// hyphens, underscores) before executing.
pub async fn run_recipe(project_dir: &Path, recipe: &str) -> Result<RunOutput, TypstError> {
    // Validate recipe name: only allow alphanumeric, hyphens, underscores.
    if recipe.is_empty()
        || !recipe
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err(TypstError::InvalidRequest {
            message: format!(
                "invalid recipe name: '{recipe}' (only alphanumeric, hyphens, underscores allowed)"
            ),
        });
    }

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        Command::new("just")
            .arg(recipe)
            .current_dir(project_dir)
            .output(),
    )
    .await
    .map_err(|_| TypstError::CommandFailed {
        message: format!("just {recipe}: timed out after 120 seconds"),
    })?
    .map_err(|e| TypstError::CommandFailed {
        message: format!("failed to run just {recipe}: {e}"),
    })?;

    Ok(RunOutput {
        exit_code: output.status.code().unwrap_or(-1),
        stdout:    String::from_utf8_lossy(&output.stdout).to_string(),
        stderr:    String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

/// Run an arbitrary shell command in the project directory.
///
/// The command is executed via `sh -c` with a 120-second timeout.
pub async fn run_command(project_dir: &Path, command: &str) -> Result<RunOutput, TypstError> {
    if command.trim().is_empty() {
        return Err(TypstError::InvalidRequest {
            message: "command must not be empty".to_owned(),
        });
    }

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        Command::new("sh")
            .args(["-c", command])
            .current_dir(project_dir)
            .output(),
    )
    .await
    .map_err(|_| TypstError::CommandFailed {
        message: format!("command timed out after 120 seconds: {command}"),
    })?
    .map_err(|e| TypstError::CommandFailed {
        message: format!("command failed: {e}"),
    })?;

    Ok(RunOutput {
        exit_code: output.status.code().unwrap_or(-1),
        stdout:    String::from_utf8_lossy(&output.stdout).to_string(),
        stderr:    String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_justfile_returns_false_for_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!has_justfile(dir.path()));
    }

    #[test]
    fn has_justfile_returns_true_when_justfile_exists() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("justfile"), "build:\n\techo ok").unwrap();
        assert!(has_justfile(dir.path()));
    }

    #[test]
    fn has_justfile_detects_capitalized_variant() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Justfile"), "build:\n\techo ok").unwrap();
        assert!(has_justfile(dir.path()));
    }

    #[tokio::test]
    async fn run_recipe_rejects_invalid_name() {
        let dir = tempfile::tempdir().unwrap();
        let result = run_recipe(dir.path(), "foo;bar").await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("invalid recipe name"));
    }

    #[tokio::test]
    async fn run_command_rejects_empty() {
        let dir = tempfile::tempdir().unwrap();
        let result = run_command(dir.path(), "").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn run_command_executes_simple_command() {
        let dir = tempfile::tempdir().unwrap();
        let output = run_command(dir.path(), "echo hello").await.unwrap();
        assert_eq!(output.exit_code, 0);
        assert!(output.stdout.contains("hello"));
    }
}
