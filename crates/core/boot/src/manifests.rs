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

//! Agent manifest loading utilities.

use std::path::Path;

use rara_agents::AgentRegistry;
use rara_kernel::process::manifest_loader::ManifestLoader;
use tracing::info;

/// Load agent manifests from the predefined [`AgentRegistry`], then overlay
/// user-defined agents from the default data directory (`<data_dir>/agents`).
///
/// The loading order is:
/// 1. Kernel bundled defaults (YAML, for backward compatibility)
/// 2. [`AgentRegistry`] entries (Rust-defined, overrides bundled YAML)
/// 3. User directory YAML files (overrides everything above)
pub fn load_default_manifests() -> ManifestLoader {
    let mut loader = ManifestLoader::new();
    loader.load_bundled();
    loader.load_manifests(AgentRegistry::all().into_iter().map(|e| e.manifest));
    let user_dir = rara_paths::data_dir().join("agents");
    let _ = loader.load_dir(&user_dir);
    info!(count = loader.list().len(), "agent manifests loaded");
    loader
}

/// Load agent manifests from the predefined [`AgentRegistry`], then overlay
/// user-defined agents from a custom directory.
pub fn load_manifests_from(dir: &Path) -> ManifestLoader {
    let mut loader = ManifestLoader::new();
    loader.load_bundled();
    loader.load_manifests(AgentRegistry::all().into_iter().map(|e| e.manifest));
    let _ = loader.load_dir(dir);
    loader
}
