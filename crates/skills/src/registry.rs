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

//! Skill registry for managing discovered and installed skills.
//!
//! Provides the [`SkillRegistry`] async trait and [`InMemoryRegistry`], a
//! cheaply-cloneable, thread-safe registry backed by `Arc<RwLock<HashMap>>`.

use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, RwLock},
};

use async_trait::async_trait;
use snafu::ResultExt;

use crate::{
    discover::SkillDiscoverer,
    error::{IoSnafu, NotAllowedSnafu, NotFoundSnafu, Result},
    parse,
    types::{SkillContent, SkillMetadata},
};

/// Registry for managing discovered and installed skills.
#[async_trait]
pub trait SkillRegistry: Send + Sync {
    /// List metadata for all available skills.
    async fn list_skills(&self) -> Result<Vec<SkillMetadata>>;

    /// Load the full content of a skill by name.
    async fn load_skill(&self, name: &str) -> Result<SkillContent>;

    /// Install a skill from a source (e.g. git URL).
    async fn install_skill(&self, source: &str) -> Result<SkillMetadata>;

    /// Remove an installed skill by name.
    async fn remove_skill(&self, name: &str) -> Result<()>;
}

/// Cheaply-cloneable in-memory skill registry.
///
/// All clones share the same underlying `HashMap` via `Arc<RwLock<…>>`,
/// so there is no need for an outer `Arc<RwLock<InMemoryRegistry>>`.
#[derive(Clone)]
pub struct InMemoryRegistry {
    skills: Arc<RwLock<HashMap<String, SkillMetadata>>>,
}

impl InMemoryRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            skills: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Populate the registry from a discoverer.
    pub async fn from_discoverer(discoverer: &dyn SkillDiscoverer) -> Result<Self> {
        let discovered = discoverer.discover().await?;
        let mut skills = HashMap::new();
        for meta in discovered {
            skills.insert(meta.name.clone(), meta);
        }
        Ok(Self {
            skills: Arc::new(RwLock::new(skills)),
        })
    }

    /// Add a skill directly.
    pub fn insert(&self, meta: SkillMetadata) {
        self.skills.write().unwrap().insert(meta.name.clone(), meta);
    }

    /// List all skill metadata.
    pub fn list_all(&self) -> Vec<SkillMetadata> {
        self.skills.read().unwrap().values().cloned().collect()
    }

    /// Get a clone of a skill's metadata by name.
    pub fn get(&self, name: &str) -> Option<SkillMetadata> {
        self.skills.read().unwrap().get(name).cloned()
    }

    /// Remove a skill by name. Returns the removed metadata, if any.
    pub fn remove(&self, name: &str) -> Option<SkillMetadata> {
        self.skills.write().unwrap().remove(name)
    }
}

impl Default for InMemoryRegistry {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl SkillRegistry for InMemoryRegistry {
    async fn list_skills(&self) -> Result<Vec<SkillMetadata>> { Ok(self.list_all()) }

    async fn load_skill(&self, name: &str) -> Result<SkillContent> {
        let meta = self
            .get(name)
            .ok_or_else(|| NotFoundSnafu { name }.build())?;

        let skill_md = meta.path.join("SKILL.md");
        let content = tokio::fs::read_to_string(&skill_md)
            .await
            .context(IoSnafu)?;
        parse::parse_skill(&content, &meta.path)
    }

    async fn install_skill(&self, _source: &str) -> Result<SkillMetadata> {
        NotAllowedSnafu {
            message: "install not supported on in-memory registry; use install::install_skill",
        }
        .fail()
    }

    async fn remove_skill(&self, name: &str) -> Result<()> {
        let meta = self
            .get(name)
            .ok_or_else(|| NotFoundSnafu { name }.build())?;

        let path = &meta.path;
        if !path.exists() {
            return NotAllowedSnafu {
                message: format!("skill directory does not exist: {}", path.display()),
            }
            .fail();
        }

        // Only allow removing registry-installed skills
        if meta.source != Some(crate::types::SkillSource::Registry) {
            return NotAllowedSnafu {
                message: format!(
                    "can only remove registry-installed skills, '{}' is {:?}",
                    name, meta.source
                ),
            }
            .fail();
        }

        tokio::fs::remove_dir_all(path).await.context(IoSnafu)?;
        // Remove from in-memory map so list_skills() no longer returns it
        self.remove(name);
        Ok(())
    }
}

/// Convenience: load a skill's full content given its path.
pub async fn load_skill_from_path(skill_dir: &Path) -> Result<SkillContent> {
    let skill_md = skill_dir.join("SKILL.md");
    let content = tokio::fs::read_to_string(&skill_md)
        .await
        .context(IoSnafu)?;
    parse::parse_skill(&content, skill_dir)
}
