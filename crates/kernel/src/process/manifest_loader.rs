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

//! ManifestLoader — loads [`AgentManifest`] definitions.
//!
//! Supports two sources:
//! - **Code-defined**: loaded via
//!   [`load_manifests`](ManifestLoader::load_manifests)
//! - **User directory**: YAML files loaded at runtime from a filesystem path

use std::path::Path;

use snafu::ResultExt;
use tracing::warn;

use super::AgentManifest;
use crate::error::{IoSnafu, Result};

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
        let entries = std::fs::read_dir(dir).context(IoSnafu)?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|ext| ext == "yaml" || ext == "yml")
            {
                let content = std::fs::read_to_string(&path).context(IoSnafu)?;
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
