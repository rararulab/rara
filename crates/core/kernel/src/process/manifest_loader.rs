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

//! ManifestLoader — loads [`AgentManifest`] definitions from YAML files.
//!
//! Supports two sources:
//! - **Bundled**: compiled into the binary via `include_str!`
//! - **User directory**: loaded at runtime from a filesystem path

use std::path::Path;

use tracing::warn;

use super::AgentManifest;
use crate::error::{KernelError, Result};

/// Loads [`AgentManifest`] definitions from YAML files.
///
/// Manifests are identified by name. Later loads override earlier ones with
/// the same name, enabling user-defined overrides of bundled defaults.
pub struct ManifestLoader {
    manifests: Vec<AgentManifest>,
}

impl ManifestLoader {
    /// Create an empty loader.
    pub fn new() -> Self {
        Self {
            manifests: Vec::new(),
        }
    }

    /// Load all bundled agent manifests (compiled into the binary).
    pub fn load_bundled(&mut self) {
        let sources = [
            include_str!("defaults/scout.yaml"),
            include_str!("defaults/planner.yaml"),
            include_str!("defaults/worker.yaml"),
        ];
        for src in sources {
            match serde_yaml::from_str::<AgentManifest>(src) {
                Ok(m) => self.manifests.push(m),
                Err(e) => warn!(error = %e, "failed to parse bundled agent manifest"),
            }
        }
    }

    /// Load user-defined manifests from a directory.
    ///
    /// Later loads override earlier ones with the same name, allowing users
    /// to customize bundled agent definitions.
    ///
    /// Returns the number of manifests successfully loaded.
    pub fn load_dir(&mut self, dir: &Path) -> Result<usize> {
        if !dir.is_dir() {
            return Ok(0);
        }
        let mut count = 0;
        let entries = std::fs::read_dir(dir).map_err(|e| KernelError::IO {
            source:   e,
            location: snafu::Location::new(file!(), line!(), 0),
        })?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|ext| ext == "yaml" || ext == "yml")
            {
                let content = std::fs::read_to_string(&path).map_err(|e| KernelError::IO {
                    source:   e,
                    location: snafu::Location::new(file!(), line!(), 0),
                })?;
                match serde_yaml::from_str::<AgentManifest>(&content) {
                    Ok(m) => {
                        // Override existing manifest with same name
                        self.manifests.retain(|existing| existing.name != m.name);
                        self.manifests.push(m);
                        count += 1;
                    }
                    Err(e) => {
                        warn!(
                            path = %path.display(),
                            error = %e,
                            "skipping invalid agent manifest"
                        );
                    }
                }
            }
        }
        Ok(count)
    }

    /// Get a manifest by name.
    pub fn get(&self, name: &str) -> Option<&AgentManifest> {
        self.manifests.iter().find(|m| m.name == name)
    }

    /// List all loaded manifests.
    pub fn list(&self) -> &[AgentManifest] { &self.manifests }
}

impl Default for ManifestLoader {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn test_manifest_loader_bundled() {
        let mut loader = ManifestLoader::new();
        loader.load_bundled();

        assert_eq!(loader.list().len(), 3);
        assert!(loader.get("scout").is_some());
        assert!(loader.get("planner").is_some());
        assert!(loader.get("worker").is_some());
        assert!(loader.get("nonexistent").is_none());

        let scout = loader.get("scout").unwrap();
        assert_eq!(scout.model, "deepseek/deepseek-chat");
        assert!(scout.tools.contains(&"read_file".to_string()));
        assert!(scout.tools.contains(&"grep".to_string()));
        assert_eq!(scout.max_iterations, Some(15));
    }

    #[test]
    fn test_manifest_loader_dir() {
        let dir = std::env::temp_dir().join("manifest_loader_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // Write a custom manifest
        fs::write(
            dir.join("custom.yaml"),
            r#"
name: custom-agent
description: "A custom agent for testing"
model: "gpt-4"
system_prompt: "You are a test agent."
tools:
  - read_file
max_iterations: 5
"#,
        )
        .unwrap();

        // Write a non-yaml file (should be skipped)
        fs::write(dir.join("readme.txt"), "not a manifest").unwrap();

        let mut loader = ManifestLoader::new();
        let count = loader.load_dir(&dir).unwrap();
        assert_eq!(count, 1);

        let custom = loader.get("custom-agent").unwrap();
        assert_eq!(custom.model, "gpt-4");
        assert_eq!(custom.max_iterations, Some(5));

        // Cleanup
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_manifest_loader_dir_override() {
        let dir = std::env::temp_dir().join("manifest_loader_override_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // Write a scout override
        fs::write(
            dir.join("scout.yaml"),
            r#"
name: scout
description: "Overridden scout"
model: "gpt-4-turbo"
system_prompt: "You are an overridden scout."
tools:
  - read_file
max_iterations: 30
"#,
        )
        .unwrap();

        let mut loader = ManifestLoader::new();
        loader.load_bundled();
        assert_eq!(loader.get("scout").unwrap().model, "deepseek/deepseek-chat");

        let count = loader.load_dir(&dir).unwrap();
        assert_eq!(count, 1);

        // Scout should be overridden
        let scout = loader.get("scout").unwrap();
        assert_eq!(scout.model, "gpt-4-turbo");
        assert_eq!(scout.max_iterations, Some(30));

        // Other bundled manifests remain
        assert!(loader.get("planner").is_some());
        assert!(loader.get("worker").is_some());

        // Cleanup
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_manifest_loader_nonexistent_dir() {
        let mut loader = ManifestLoader::new();
        let count = loader
            .load_dir(Path::new("/nonexistent/path/that/does/not/exist"))
            .unwrap();
        assert_eq!(count, 0);
    }
}
