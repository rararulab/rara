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

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use dashmap::DashMap;
use snafu::ResultExt;

use super::AgentManifest;
use crate::error::{IoSnafu, KernelError, Result};

/// Shared reference to the [`AgentRegistry`].
pub type AgentRegistryRef = Arc<AgentRegistry>;

pub struct AgentRegistry {
    builtin:    HashMap<String, AgentManifest>,
    custom:     DashMap<String, AgentManifest>,
    agents_dir: PathBuf,
}

impl AgentRegistry {
    pub fn new(builtin: Vec<AgentManifest>, agents_dir: PathBuf) -> Self {
        let builtin = builtin.into_iter().map(|m| (m.name.clone(), m)).collect();
        Self {
            builtin,
            custom: DashMap::new(),
            agents_dir,
        }
    }

    pub fn init(
        builtin: Vec<AgentManifest>,
        loader: &super::manifest_loader::ManifestLoader,
        agents_dir: PathBuf,
    ) -> Self {
        let registry = Self::new(builtin, agents_dir);
        for manifest in loader.list() {
            let name = manifest.name.clone();
            // Only add to custom if not already a builtin
            if !registry.builtin.contains_key(&name) {
                registry.custom.insert(name, manifest.clone());
            }
        }
        registry
    }

    #[tracing::instrument(skip(self))]
    pub fn get(&self, name: &str) -> Option<AgentManifest> {
        // Custom first (shadow), then builtin
        if let Some(m) = self.custom.get(name) {
            return Some(m.value().clone());
        }
        self.builtin.get(name).cloned()
    }

    pub fn list(&self) -> Vec<AgentManifest> {
        let mut result: HashMap<String, AgentManifest> = self.builtin.clone();
        for entry in self.custom.iter() {
            result.insert(entry.key().clone(), entry.value().clone());
        }
        result.into_values().collect()
    }

    #[tracing::instrument(skip(self, manifest), fields(agent_name = %manifest.name))]
    pub fn register(&self, manifest: AgentManifest) -> Result<()> {
        let name = manifest.name.clone();
        // Persist to YAML
        let path = self.agents_dir.join(format!("{}.yaml", name));
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let yaml = serde_yaml::to_string(&manifest)
            .whatever_context::<_, KernelError>("failed to serialize manifest")?;
        std::fs::write(&path, yaml).context(IoSnafu)?;
        self.custom.insert(name, manifest);
        Ok(())
    }

    pub fn unregister(&self, name: &str) -> Result<()> {
        if self.builtin.contains_key(name) {
            return Err(KernelError::Other {
                message: format!("cannot unregister builtin agent: {name}").into(),
            });
        }
        self.custom.remove(name);
        let path = self.agents_dir.join(format!("{}.yaml", name));
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
        Ok(())
    }

    pub fn is_builtin(&self, name: &str) -> bool { self.builtin.contains_key(name) }

    pub fn agents_dir(&self) -> &Path { &self.agents_dir }
}
