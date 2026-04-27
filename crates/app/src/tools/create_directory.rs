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

//! Directory creation primitive.
//!
//! Path resolution goes through [`super::path_check::resolve_writable`] so
//! both absolute-path escape and symlink escape inside the workspace are
//! rejected before any host mkdir happens (see #1936).

use anyhow::Context;
use async_trait::async_trait;
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::path_check::resolve_writable;

/// Input parameters for the create-directory tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateDirectoryParams {
    /// Path of the directory to create.
    path: String,
}

/// Typed result returned by the create-directory tool.
#[derive(Debug, Clone, Serialize)]
pub struct CreateDirectoryResult {
    /// The resolved path of the created directory.
    pub path:            String,
    /// Whether parent directories were created.
    pub created_parents: bool,
}

/// Layer 1 primitive: create a directory on the filesystem.
#[derive(ToolDef)]
#[tool(
    name = "create-directory",
    description = "Create a directory on the filesystem. Automatically creates parent directories \
                   if they do not exist.",
    tier = "deferred"
)]
pub struct CreateDirectoryTool;

impl CreateDirectoryTool {
    /// Create a new instance.
    pub fn new() -> Self { Self }
}

#[async_trait]
impl ToolExecute for CreateDirectoryTool {
    type Output = CreateDirectoryResult;
    type Params = CreateDirectoryParams;

    async fn run(
        &self,
        params: CreateDirectoryParams,
        _context: &ToolContext,
    ) -> anyhow::Result<CreateDirectoryResult> {
        let dir_path = resolve_writable(&params.path).await?;

        // Check if parent needs to be created to report `created_parents`.
        let parent_exists = dir_path.parent().map_or(true, |p| p.exists());

        tokio::fs::create_dir_all(&dir_path)
            .await
            .context(format!("failed to create directory {}", dir_path.display()))?;

        Ok(CreateDirectoryResult {
            path:            dir_path.display().to_string(),
            created_parents: !parent_exists,
        })
    }
}
