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

//! ManifestLoader — loads [`AgentManifest`] definitions.
//!
//! Supports two sources:
//! - **Code-defined**: loaded via [`load_manifests`](ManifestLoader::load_manifests)
//! - **User directory**: YAML files loaded at runtime from a filesystem path

use std::path::Path;

use tracing::warn;

use super::AgentManifest;
use crate::error::{KernelError, Result};

/// Loads [`AgentManifest`] definitions.
///
/// Manifests are identified by name. Later loads override earlier ones with
/// the same name, enabling user-defined overrides of code-defined defaults.
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

    /// Load user-defined manifests from a directory.
    ///
    /// Later loads override earlier ones with the same name, allowing users
    /// to customize code-defined agent definitions.
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

    /// Load manifests from code-defined sources.
    ///
    /// Each manifest is inserted by name. If a manifest with the same name
    /// already exists, it is replaced (last-write-wins).
    pub fn load_manifests(&mut self, manifests: impl IntoIterator<Item = AgentManifest>) {
        for manifest in manifests {
            self.manifests.retain(|m| m.name != manifest.name);
            self.manifests.push(manifest);
        }
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
    fn test_manifest_loader_load_manifests() {
        let mut loader = ManifestLoader::new();
        let manifest = AgentManifest {
            name: "test-agent".to_string(),
        role:           None,
            description: "test".to_string(),
            model: "gpt-4".to_string(),
            system_prompt: "You are a test agent.".to_string(),
            soul_prompt:    None,
            provider_hint: None,
            max_iterations: Some(5),
            tools: vec![],
            max_children: None,
            max_context_tokens: None,
            priority: Default::default(),
            metadata: Default::default(),
            sandbox: None,
        };
        loader.load_manifests(std::iter::once(manifest));
        assert_eq!(loader.list().len(), 1);
        assert_eq!(loader.get("test-agent").unwrap().model, "gpt-4");
    }

    #[test]
    fn test_manifest_loader_override() {
        let mut loader = ManifestLoader::new();
        let m1 = AgentManifest {
            name: "agent".to_string(),
        role:           None,
            description: "v1".to_string(),
            model: "gpt-3.5".to_string(),
            system_prompt: "v1".to_string(),
            soul_prompt:    None,
            provider_hint: None,
            max_iterations: Some(5),
            tools: vec![],
            max_children: None,
            max_context_tokens: None,
            priority: Default::default(),
            metadata: Default::default(),
            sandbox: None,
        };
        let m2 = AgentManifest {
            name: "agent".to_string(),
        role:           None,
            description: "v2".to_string(),
            model: "gpt-4".to_string(),
            system_prompt: "v2".to_string(),
            soul_prompt:    None,
            provider_hint: None,
            max_iterations: Some(10),
            tools: vec![],
            max_children: None,
            max_context_tokens: None,
            priority: Default::default(),
            metadata: Default::default(),
            sandbox: None,
        };
        loader.load_manifests(std::iter::once(m1));
        loader.load_manifests(std::iter::once(m2));
        assert_eq!(loader.list().len(), 1);
        assert_eq!(loader.get("agent").unwrap().model, "gpt-4");
    }

    #[test]
    fn test_manifest_loader_dir() {
        let dir = std::env::temp_dir().join("manifest_loader_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

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

        fs::write(dir.join("readme.txt"), "not a manifest").unwrap();

        let mut loader = ManifestLoader::new();
        let count = loader.load_dir(&dir).unwrap();
        assert_eq!(count, 1);

        let custom = loader.get("custom-agent").unwrap();
        assert_eq!(custom.model, "gpt-4");
        assert_eq!(custom.max_iterations, Some(5));

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
