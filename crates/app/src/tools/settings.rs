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

//! Settings tool for runtime configuration introspection and mutation.

use std::sync::Arc;

use async_trait::async_trait;
use rara_backend_admin::settings::SettingsSvc;
use rara_domain_shared::settings::SettingsProvider;
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

const SENSITIVE_FRAGMENTS: &[&str] = &["api_key", "token", "password", "secret"];

/// Number of version entries returned by the history action.
const TOOL_HISTORY_LIMIT: i64 = 20;

/// Number of leading characters shown before masking a sensitive value.
const MASK_VISIBLE_LEN: usize = 6;

/// Available actions for the settings tool.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum SettingsAction {
    /// List all settings.
    List,
    /// Get a single setting by key.
    Get,
    /// Set a single setting by key.
    Set,
    /// Show the current settings version number.
    Version,
    /// Show recent version history.
    History,
    /// Show a point-in-time snapshot at a given version.
    Snapshot,
    /// Rollback settings to a given version.
    Rollback,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SettingsParams {
    /// The action to perform.
    action:  SettingsAction,
    /// The setting key (required for get/set).
    key:     Option<String>,
    /// The value to set (required for set).
    value:   Option<String>,
    /// The version number (required for snapshot/rollback).
    version: Option<i64>,
}

/// Agent tool that reads and modifies runtime settings.
#[derive(ToolDef)]
#[tool(
    name = "settings",
    description = "Read and modify runtime settings. Actions: 'list' to see all, 'get' to read a \
                   key, 'set' to update a value, 'version' for current version, 'history' for \
                   recent changes, 'snapshot' for point-in-time view, 'rollback' to revert to a \
                   version.",
    tier = "deferred"
)]
pub struct SettingsTool {
    settings: Arc<SettingsSvc>,
}
impl SettingsTool {
    /// Create a new settings tool backed by the MVCC settings service.
    pub fn new(settings: Arc<SettingsSvc>) -> Self { Self { settings } }
}

#[async_trait]
impl ToolExecute for SettingsTool {
    type Output = Value;
    type Params = SettingsParams;

    async fn run(&self, params: SettingsParams, _context: &ToolContext) -> anyhow::Result<Value> {
        match params.action {
            SettingsAction::List => {
                let all = self.settings.list().await;
                let masked: serde_json::Map<String, Value> = all
                    .into_iter()
                    .map(|(k, v)| {
                        let display = maybe_mask(&k, &v);
                        (k, Value::String(display))
                    })
                    .collect();
                Ok(json!({"settings": masked}))
            }
            SettingsAction::Get => {
                let key = params
                    .key
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("missing required parameter: key"))?;
                match self.settings.get(key).await {
                    Some(value) => Ok(json!({"key": key, "value": maybe_mask(key, &value)})),
                    None => Ok(json!({"key": key, "value": null})),
                }
            }
            SettingsAction::Set => {
                let key = params
                    .key
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("missing required parameter: key"))?;
                let value = params
                    .value
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("missing required parameter: value"))?;
                self.settings.set(key, value).await?;
                Ok(json!({"key": key, "updated": true}))
            }
            SettingsAction::Version => {
                let ver = self.settings.current_version().await?;
                Ok(json!({"version": ver}))
            }
            SettingsAction::History => {
                let entries = self.settings.list_versions(TOOL_HISTORY_LIMIT).await?;
                let masked: Vec<Value> = entries
                    .into_iter()
                    .map(|e| {
                        let masked_val = e.value.as_deref().map(|v| maybe_mask(&e.key, v));
                        json!({
                            "version": e.version,
                            "key": e.key,
                            "value": masked_val,
                            "changed_at": e.changed_at,
                        })
                    })
                    .collect();
                Ok(json!({"versions": masked}))
            }
            SettingsAction::Snapshot => {
                let ver = params
                    .version
                    .ok_or_else(|| anyhow::anyhow!("missing required parameter: version"))?;
                let snap = self.settings.snapshot(ver).await?;
                let masked: serde_json::Map<String, Value> = snap
                    .into_iter()
                    .map(|(k, v)| {
                        let display = maybe_mask(&k, &v);
                        (k, Value::String(display))
                    })
                    .collect();
                Ok(json!({"version": ver, "settings": masked}))
            }
            SettingsAction::Rollback => {
                let ver = params
                    .version
                    .ok_or_else(|| anyhow::anyhow!("missing required parameter: version"))?;
                let new_ver = self.settings.rollback_to(ver).await?;
                Ok(json!({"rolled_back_to": ver, "new_version": new_ver}))
            }
        }
    }
}

fn maybe_mask(key: &str, value: &str) -> String {
    let key_lower = key.to_ascii_lowercase();
    let is_sensitive = SENSITIVE_FRAGMENTS
        .iter()
        .any(|frag| key_lower.contains(frag));
    if is_sensitive {
        // Use char iterator to avoid panic on multi-byte UTF-8 values.
        let char_count = value.chars().count();
        if char_count < MASK_VISIBLE_LEN {
            "****".to_owned()
        } else {
            let prefix: String = value.chars().take(MASK_VISIBLE_LEN).collect();
            format!("{prefix}****")
        }
    } else {
        value.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mask_sensitive_keys() {
        assert_eq!(
            maybe_mask("llm.providers.openrouter.api_key", "sk-or-v1-abc123"),
            "sk-or-****"
        );
        assert_eq!(
            maybe_mask("telegram.bot_token", "12345:ABCDE"),
            "12345:****"
        );
        assert_eq!(maybe_mask("gmail.app_password", "abcd"), "****");
        assert_eq!(maybe_mask("some.secret.value", "longvalue"), "longva****");
    }
    #[test]
    fn non_sensitive_keys_not_masked() {
        assert_eq!(
            maybe_mask("llm.default_provider", "openrouter"),
            "openrouter"
        );
        assert_eq!(
            maybe_mask("gmail.address", "me@example.com"),
            "me@example.com"
        );
    }
}
