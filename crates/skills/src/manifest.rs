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

use fs2::FileExt;
use snafu::ResultExt;

use crate::{
    error::{IoSnafu, Result, SerdeJsonSnafu},
    types::SkillsManifest,
};

/// Persistent manifest storage with atomic writes and file-level locking.
///
/// Prefer [`with_lock`](Self::with_lock) for all manifest mutations.
/// Bare [`load`](Self::load) / [`save`](Self::save) remain available for
/// read-only access and backward compatibility.
pub struct ManifestStore {
    path: PathBuf,
}

impl ManifestStore {
    /// Create a store backed by the given JSON file path.
    pub fn new(path: PathBuf) -> Self { Self { path } }

    /// Default manifest location under the data directory.
    pub fn default_path() -> Result<PathBuf> {
        Ok(rara_paths::data_dir().join("skills-manifest.json"))
    }

    /// Execute `f` while holding an exclusive file lock on the manifest.
    ///
    /// The lock file is `<manifest>.lock` (e.g. `skills-manifest.json.lock`).
    /// The manifest is loaded before calling `f`, and saved after `f` returns
    /// `Ok`. The lock is released when the `File` guard drops.
    ///
    /// Uses `flock()` via `fs2` — advisory only on NFS, but sufficient for
    /// single-host CLI usage.
    pub fn with_lock<T, F>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&mut SkillsManifest) -> Result<T>,
    {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).context(IoSnafu)?;
        }

        let lock_path = self.path.with_extension("json.lock");
        let lock_file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .context(IoSnafu)?;

        lock_file.lock_exclusive().context(IoSnafu)?;

        // From here the lock is held; it releases on `lock_file` drop.
        let mut manifest = self.load()?;
        let result = f(&mut manifest)?;
        self.save(&manifest)?;
        Ok(result)
    }

    /// Load manifest from disk, returning a default if missing.
    ///
    /// For read-only access this is fine; for mutations prefer
    /// [`with_lock`](Self::with_lock).
    pub fn load(&self) -> Result<SkillsManifest> {
        if !self.path.exists() {
            return Ok(SkillsManifest::default());
        }
        let data = std::fs::read_to_string(&self.path).context(IoSnafu)?;
        let manifest: SkillsManifest = serde_json::from_str(&data).context(SerdeJsonSnafu)?;
        Ok(manifest)
    }

    /// Save manifest atomically via temp file + rename.
    ///
    /// For mutations prefer [`with_lock`](Self::with_lock) which handles
    /// load + save under an exclusive lock.
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

    /// Path to the backing JSON file.
    pub fn path(&self) -> &Path { &self.path }
}
