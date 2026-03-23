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

//! System paths tool — exposes rara's directory layout to LLM agents.
//!
//! Returns the resolved absolute paths for all standard rara directories
//! (config, data, workspace, logs, etc.) so agents can construct correct
//! file paths without guessing.

use async_trait::async_trait;
use rara_kernel::tool::{EmptyParams, ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use serde::Serialize;

/// Resolved directory paths returned by the system-paths tool.
#[derive(Debug, Clone, Serialize)]
pub struct SystemPathsResult {
    /// Home directory of the user running rara.
    pub home:        String,
    /// Root configuration directory (`~/.config/rara` on macOS).
    pub config:      String,
    /// Root data directory (`~/Library/Application Support/rara` on macOS).
    pub data:        String,
    /// Temporary / cache directory.
    pub temp:        String,
    /// Log files directory.
    pub logs:        String,
    /// Agent workspace root — the primary sandbox for file operations.
    pub workspace:   String,
    /// User-editable prompt files.
    pub prompts:     String,
    /// User skills directory.
    pub skills:      String,
    /// Session JSONL storage.
    pub sessions:    String,
    /// Memory documents directory.
    pub memory:      String,
    /// Tool-produced artifacts (images, resources).
    pub resources:   String,
    /// Database directory.
    pub database:    String,
    /// YAML configuration file path.
    pub config_file: String,
}

/// Tool that returns the resolved paths for all standard rara directories.
///
/// LLM agents should call this tool instead of guessing absolute paths,
/// which vary by platform and deployment (e.g. `/Users/rara` on a server
/// vs `/Users/ryan` locally).
#[derive(ToolDef)]
#[tool(
    name = "system-paths",
    description = "Returns the absolute paths of all rara system directories (config, data, \
                   workspace, logs, skills, etc.). Call this before constructing file paths to \
                   avoid hardcoding incorrect directories.",
    tier = "deferred"
)]
pub struct SystemPathsTool;

impl SystemPathsTool {
    /// Create a new instance.
    pub fn new() -> Self { Self }
}

#[async_trait]
impl ToolExecute for SystemPathsTool {
    type Output = SystemPathsResult;
    type Params = EmptyParams;

    async fn run(
        &self,
        _params: EmptyParams,
        _context: &ToolContext,
    ) -> anyhow::Result<SystemPathsResult> {
        Ok(SystemPathsResult {
            home:        rara_paths::home_dir().display().to_string(),
            config:      rara_paths::config_dir().display().to_string(),
            data:        rara_paths::data_dir().display().to_string(),
            temp:        rara_paths::temp_dir().display().to_string(),
            logs:        rara_paths::logs_dir().display().to_string(),
            workspace:   rara_paths::workspace_dir().display().to_string(),
            prompts:     rara_paths::prompts_dir().display().to_string(),
            skills:      rara_paths::skills_dir().display().to_string(),
            sessions:    rara_paths::sessions_dir().display().to_string(),
            memory:      rara_paths::memory_dir().display().to_string(),
            resources:   rara_paths::resources_dir().display().to_string(),
            database:    rara_paths::database_dir().display().to_string(),
            config_file: rara_paths::config_file().display().to_string(),
        })
    }
}
