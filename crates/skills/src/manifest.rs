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

//! Persistent manifest storage for installed skill repos.
//!
//! [`ManifestStore`] reads and writes a JSON [`SkillsManifest`]
//! to disk using atomic writes (write to `.tmp` file, then rename) to avoid
//! corruption.

use std::path::{Path, PathBuf};

use snafu::ResultExt;

use crate::{
    error::{IoSnafu, Result, SerdeJsonSnafu},
    types::SkillsManifest,
};

/// Persistent manifest storage with atomic writes.
pub struct ManifestStore {
    path: PathBuf,
}

impl ManifestStore {
    pub fn new(path: PathBuf) -> Self { Self { path } }

    pub fn default_path() -> Result<PathBuf> {
        Ok(rara_paths::data_dir().join("skills-manifest.json"))
    }

    /// Load manifest from disk, returning a default if missing.
    pub fn load(&self) -> Result<SkillsManifest> {
        if !self.path.exists() {
            return Ok(SkillsManifest::default());
        }
        let data = std::fs::read_to_string(&self.path).context(IoSnafu)?;
        let manifest: SkillsManifest = serde_json::from_str(&data).context(SerdeJsonSnafu)?;
        Ok(manifest)
    }

    /// Save manifest atomically via temp file + rename.
    pub fn save(&self, manifest: &SkillsManifest) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).context(IoSnafu)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        let data = serde_json::to_string_pretty(manifest).context(SerdeJsonSnafu)?;
        std::fs::write(&tmp, data).context(IoSnafu)?;
        std::fs::rename(&tmp, &self.path).context(IoSnafu)?;
        Ok(())
    }

    pub fn path(&self) -> &Path { &self.path }
}
