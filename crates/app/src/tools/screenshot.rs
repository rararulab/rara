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

//! Layer 2 service tool for capturing web page screenshots via Playwright.

use std::path::PathBuf;

use async_trait::async_trait;
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tracing::info;
use uuid::Uuid;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScreenshotParams {
    /// The URL to screenshot.
    url:       String,
    /// CSS selector to screenshot a specific element.
    selector:  Option<String>,
    /// Viewport width in pixels (default: 1280).
    width:     Option<u64>,
    /// Viewport height in pixels (default: 720).
    height:    Option<u64>,
    /// Capture full scrollable page (default: false).
    full_page: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScreenshotResult {
    pub success: bool,
    pub path:    String,
}

#[derive(ToolDef)]
#[tool(
    name = "screenshot",
    description = "Take a screenshot of a web page using Playwright. Useful for previewing \
                   frontend work, checking UI changes, or sharing visual results."
)]
pub struct ScreenshotTool {
    project_root: PathBuf,
}
impl ScreenshotTool {
    pub fn new(project_root: PathBuf) -> Self { Self { project_root } }
}

#[async_trait]
impl ToolExecute for ScreenshotTool {
    type Output = ScreenshotResult;
    type Params = ScreenshotParams;

    async fn run(
        &self,
        params: ScreenshotParams,
        _context: &ToolContext,
    ) -> anyhow::Result<ScreenshotResult> {
        let selector = params.selector.as_deref().unwrap_or("");
        let width = params.width.unwrap_or(1280);
        let height = params.height.unwrap_or(720);
        let full_page = params.full_page.unwrap_or(false);
        let output_path =
            std::env::temp_dir().join(format!("rara-screenshot-{}.png", Uuid::new_v4()));
        let output_str = output_path.to_string_lossy().to_string();
        let script_path = self.project_root.join("scripts/screenshot.mjs");
        let mut cmd = tokio::process::Command::new("node");
        cmd.arg(&script_path)
            .arg(&params.url)
            .arg(&output_str)
            .arg(width.to_string())
            .arg(height.to_string())
            .arg(full_page.to_string())
            .arg(selector)
            .current_dir(&self.project_root)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        info!(url = %params.url, output = %output_str, "taking screenshot");
        let result = cmd
            .output()
            .await
            .map_err(|e| anyhow::anyhow!("failed to run screenshot script: {e}"))?;
        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr);
            return Err(anyhow::anyhow!("screenshot script failed: {stderr}"));
        }
        if !output_path.exists() {
            return Err(anyhow::anyhow!("screenshot file was not created"));
        }
        Ok(ScreenshotResult {
            success: true,
            path:    output_str,
        })
    }
}
