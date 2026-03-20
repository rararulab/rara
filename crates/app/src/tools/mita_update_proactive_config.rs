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

//! Mita-exclusive tool for dynamically updating the proactive filter
//! configuration (quiet hours, cooldowns, rate limits).
//!
//! Reads/writes `config_dir()/mita/proactive.yaml` so changes persist
//! across restarts without touching the main config file.

use std::{collections::HashMap, path::PathBuf, time::Duration};

use async_trait::async_trait;
use rara_kernel::{
    proactive::ProactiveConfig,
    tool::{ToolContext, ToolExecute},
};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::info;

use super::notify::push_notification;

/// Valid field names that can be updated.
const UPDATABLE_FIELDS: &[&str] = &["quiet_hours", "max_hourly", "cooldowns"];

/// Input parameters for the update-proactive-config tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateProactiveConfigParams {
    /// Field to update. One of: "quiet_hours", "max_hourly", "cooldowns".
    field: String,
    /// New value as JSON (will be parsed according to field type).
    value: Value,
}

/// Mita-exclusive tool: update a specific field in the proactive filter config.
#[derive(ToolDef)]
#[tool(
    name = "update-proactive-config",
    description = "Update proactive filter configuration. Adjusts quiet hours, cooldowns, or rate \
                   limits based on user preferences.",
    bypass_interceptor
)]
pub struct UpdateProactiveConfigTool;

impl UpdateProactiveConfigTool {
    /// Create a new instance.
    pub fn new() -> Self { Self }
}

/// Resolve the proactive config file path: `config_dir()/mita/proactive.yaml`.
fn config_path() -> PathBuf { rara_paths::config_dir().join("mita").join("proactive.yaml") }

/// Load the current proactive config from disk, returning `None` if the file
/// does not exist.
fn load_config() -> anyhow::Result<Option<ProactiveConfig>> {
    let path = config_path();
    if !path.exists() {
        return Ok(None);
    }
    let contents = std::fs::read_to_string(&path)?;
    let config: ProactiveConfig = serde_yaml::from_str(&contents)?;
    Ok(Some(config))
}

/// Write the proactive config to disk, creating parent directories if needed.
fn save_config(config: &ProactiveConfig) -> anyhow::Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let yaml = serde_yaml::to_string(config)?;
    std::fs::write(&path, yaml)?;
    Ok(())
}

#[async_trait]
impl ToolExecute for UpdateProactiveConfigTool {
    type Output = Value;
    type Params = UpdateProactiveConfigParams;

    async fn run(
        &self,
        params: UpdateProactiveConfigParams,
        context: &ToolContext,
    ) -> anyhow::Result<Value> {
        if !UPDATABLE_FIELDS.contains(&params.field.as_str()) {
            anyhow::bail!(
                "invalid field '{}': must be one of {}",
                params.field,
                UPDATABLE_FIELDS.join(", ")
            );
        }

        let mut config = load_config()?.ok_or_else(|| {
            anyhow::anyhow!(
                "proactive config not found at {}; cannot update a non-existent config",
                config_path().display()
            )
        })?;

        match params.field.as_str() {
            "quiet_hours" => {
                // Accept null to disable, or ["HH:MM", "HH:MM"] to set.
                let quiet: Option<(String, String)> = serde_json::from_value(params.value.clone())
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "invalid quiet_hours value: {e}. Expected null or [\"HH:MM\", \
                             \"HH:MM\"]"
                        )
                    })?;
                config.quiet_hours = quiet;
                info!(
                    quiet_hours = ?config.quiet_hours,
                    "proactive config: quiet_hours updated"
                );
            }
            "max_hourly" => {
                let max: u32 = serde_json::from_value(params.value.clone()).map_err(|e| {
                    anyhow::anyhow!("invalid max_hourly value: {e}. Expected a positive integer")
                })?;
                config.max_hourly = max;
                info!(max_hourly = max, "proactive config: max_hourly updated");
            }
            "cooldowns" => {
                // Accept a map of signal_kind -> seconds.
                let raw: HashMap<String, u64> = serde_json::from_value(params.value.clone())
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "invalid cooldowns value: {e}. Expected an object mapping signal \
                             names to seconds"
                        )
                    })?;
                // Merge into existing cooldowns rather than replacing.
                for (key, secs) in &raw {
                    config
                        .cooldowns
                        .insert(key.clone(), Duration::from_secs(*secs));
                }
                info!(
                    merged_keys = raw.len(),
                    total = config.cooldowns.len(),
                    "proactive config: cooldowns updated"
                );
            }
            _ => unreachable!(),
        }

        save_config(&config)?;

        push_notification(
            context,
            format!(
                "\u{2699}\u{fe0f} Proactive config updated: {}",
                params.field
            ),
        );

        Ok(json!({
            "status": "ok",
            "field": params.field,
            "message": format!("Proactive config field '{}' updated.", params.field)
        }))
    }
}
