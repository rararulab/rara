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

//! Soul loader — priority-chain file loading with code fallback.
//!
//! Loading priority:
//! 1. `~/.config/rara/agents/{name}/soul.md` (per-agent)
//! 2. `~/.config/rara/soul.md` (global fallback)
//! 3. Hardcoded default in code (`include_str!`)

use std::path::PathBuf;

use snafu::ResultExt;
use tracing::{debug, info};

use crate::defaults;
use crate::error::{IoSnafu, Result};
use crate::file::SoulFile;
use crate::render;
use crate::state::SoulState;

/// Source of a loaded soul definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SoulSource {
    /// Loaded from per-agent file.
    AgentFile(PathBuf),
    /// Loaded from global fallback file.
    GlobalFile(PathBuf),
    /// Using hardcoded default from code.
    CodeDefault,
}

/// Result of loading a soul: the parsed file and its source.
#[derive(Debug, Clone)]
pub struct LoadedSoul {
    pub soul:   SoulFile,
    pub source: SoulSource,
}

/// Load a soul definition for the given agent name.
///
/// Searches in priority order:
/// 1. `{config_dir}/agents/{agent_name}/soul.md` (per-agent file)
/// 2. `{config_dir}/soul.md` (global file)
/// 3. Built-in default from [`defaults::for_agent`]
///
/// Returns `None` for agents with no soul at any level (e.g. worker, mita).
pub fn load_soul(agent_name: &str) -> Result<Option<LoadedSoul>> {
    let config = rara_paths::config_dir();

    // Priority 1: per-agent soul file
    let agent_path = config.join("agents").join(agent_name).join("soul.md");
    if agent_path.exists() {
        debug!(agent = agent_name, path = %agent_path.display(), "loading per-agent soul file");
        let content = std::fs::read_to_string(&agent_path).context(IoSnafu {
            path: agent_path.clone(),
        })?;
        let soul = SoulFile::parse(&content)?;
        return Ok(Some(LoadedSoul {
            soul,
            source: SoulSource::AgentFile(agent_path),
        }));
    }

    // Priority 2: global soul file
    let global_path = config.join("soul.md");
    if global_path.exists() {
        debug!(agent = agent_name, path = %global_path.display(), "loading global soul file");
        let content = std::fs::read_to_string(&global_path).context(IoSnafu {
            path: global_path.clone(),
        })?;
        let soul = SoulFile::parse(&content)?;
        return Ok(Some(LoadedSoul {
            soul,
            source: SoulSource::GlobalFile(global_path),
        }));
    }

    // Priority 3: built-in default
    if let Some(default_content) = defaults::for_agent(agent_name) {
        debug!(agent = agent_name, "using built-in default soul definition");
        let soul = SoulFile::parse(default_content)?;
        return Ok(Some(LoadedSoul {
            soul,
            source: SoulSource::CodeDefault,
        }));
    }

    debug!(agent = agent_name, "no soul definition found");
    Ok(None)
}

/// Load the soul state for the given agent name.
///
/// State file location: `{config_dir}/agents/{agent_name}/soul-state.yaml`.
pub fn load_state(agent_name: &str) -> Result<Option<SoulState>> {
    let path = state_path(agent_name);
    SoulState::load(&path)
}

/// Save the soul state for the given agent name.
pub fn save_state(agent_name: &str, state: &SoulState) -> Result<()> {
    let path = state_path(agent_name);
    state.save(&path)
}

/// Returns the path to the per-agent soul file.
pub fn soul_path(agent_name: &str) -> PathBuf {
    rara_paths::config_dir()
        .join("agents")
        .join(agent_name)
        .join("soul.md")
}

/// Returns the path to the soul state file for an agent.
pub fn state_path(agent_name: &str) -> PathBuf {
    rara_paths::config_dir()
        .join("agents")
        .join(agent_name)
        .join("soul-state.yaml")
}

/// Returns the path to the soul snapshots directory for an agent.
pub fn snapshots_dir(agent_name: &str) -> PathBuf {
    rara_paths::config_dir()
        .join("agents")
        .join(agent_name)
        .join("soul-snapshots")
}

/// Load and render a soul prompt for the given agent.
///
/// This is the high-level convenience function that combines loading, state
/// loading, and rendering into a single call.
///
/// Returns `None` if the agent has no soul definition.
pub fn load_and_render(agent_name: &str) -> Result<Option<String>> {
    let loaded = load_soul(agent_name)?;
    let Some(loaded) = loaded else {
        return Ok(None);
    };

    info!(
        agent = agent_name,
        source = ?loaded.source,
        version = loaded.soul.frontmatter.version,
        "soul loaded"
    );

    let state = load_state(agent_name)?;
    let rendered = render::render(&loaded.soul, state.as_ref());
    Ok(Some(rendered))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_builtin_default_for_rara() {
        let loaded = load_soul("rara").unwrap().unwrap();
        assert_eq!(loaded.source, SoulSource::CodeDefault);
        assert_eq!(loaded.soul.frontmatter.name, "rara");
    }

    #[test]
    fn load_builtin_default_for_nana() {
        let loaded = load_soul("nana").unwrap().unwrap();
        assert_eq!(loaded.source, SoulSource::CodeDefault);
        assert_eq!(loaded.soul.frontmatter.name, "nana");
    }

    #[test]
    fn load_returns_none_for_unknown_agent() {
        let loaded = load_soul("nonexistent_agent_xyz").unwrap();
        assert!(loaded.is_none());
    }
}
