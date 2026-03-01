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

use rara_kernel::process::{
    agent_registry::AgentRegistry,
    manifest_loader::ManifestLoader,
};
use tracing::info;

/// Load agent manifests and build an AgentRegistry.
///
/// Builtin agents (rara, etc.) are loaded from `rara_agents`.
/// User-defined YAML files from `<data_dir>/agents` are loaded via
/// ManifestLoader and fed into the registry as custom agents.
pub fn load_default_registry() -> AgentRegistry {
    let builtin = vec![rara_agents::rara().clone()];
    let agents_dir = rara_paths::data_dir().join("agents");
    let mut loader = ManifestLoader::new();
    let _ = loader.load_dir(&agents_dir);
    let registry = AgentRegistry::init(builtin, &loader, agents_dir);
    info!(count = registry.list().len(), "agent registry initialized");
    registry
}

/// Load agent manifests from code-defined agents and a custom directory.
pub fn load_registry_from(dir: &Path) -> AgentRegistry {
    let builtin = vec![rara_agents::rara().clone()];
    let mut loader = ManifestLoader::new();
    let _ = loader.load_dir(dir);
    AgentRegistry::init(builtin, &loader, dir.to_path_buf())
}
