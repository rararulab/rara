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
//!
//! Exposes the [`SettingsProvider`] as an agent tool so rara can read and
//! modify its own runtime configuration during a conversation.

use std::sync::Arc;

use async_trait::async_trait;
use rara_domain_shared::settings::SettingsProvider;
use rara_kernel::tool::{AgentTool, ToolOutput};
use serde_json::json;

/// Sensitive-key substrings. If a settings key contains any of these
/// (case-insensitive), its value is masked in `list` and `get` responses.
const SENSITIVE_FRAGMENTS: &[&str] = &["api_key", "token", "password", "secret"];

/// Agent tool that reads and modifies runtime settings.
pub struct SettingsTool {
    settings: Arc<dyn SettingsProvider>,
}

impl SettingsTool {
    pub const NAME: &str = "settings";

    pub fn new(settings: Arc<dyn SettingsProvider>) -> Self { Self { settings } }
}

/// Mask a value if the key looks sensitive.
fn maybe_mask(key: &str, value: &str) -> String {
    let key_lower = key.to_ascii_lowercase();
    let is_sensitive = SENSITIVE_FRAGMENTS
        .iter()
        .any(|frag| key_lower.contains(frag));
    if is_sensitive {
        if value.len() < 6 {
            "****".to_owned()
        } else {
            format!("{}****", &value[..6])
        }
    } else {
        value.to_owned()
    }
}

#[async_trait]
impl AgentTool for SettingsTool {
    fn name(&self) -> &str { Self::NAME }

    fn description(&self) -> &str {
        "Read and modify runtime settings. Use 'list' to see all settings, 'get' to read a \
         specific key, 'set' to update a value."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "get", "set"],
                    "description": "The action to perform: list all settings, get a single key, or set a key to a new value"
                },
                "key": {
                    "type": "string",
                    "description": "The setting key (required for get and set)"
                },
                "value": {
                    "type": "string",
                    "description": "The value to set (required for set)"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _context: &rara_kernel::tool::ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: action"))?;

        match action {
            "list" => {
                let all = self.settings.list().await;
                let masked: serde_json::Map<String, serde_json::Value> = all
                    .into_iter()
                    .map(|(k, v)| {
                        let display = maybe_mask(&k, &v);
                        (k, serde_json::Value::String(display))
                    })
                    .collect();
                Ok(json!({ "settings": masked }).into())
            }
            "get" => {
                let key = params
                    .get("key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("missing required parameter: key"))?;
                match self.settings.get(key).await {
                    Some(value) => {
                        let display = maybe_mask(key, &value);
                        Ok(json!({ "key": key, "value": display }).into())
                    }
                    None => Ok(json!({ "key": key, "value": null }).into()),
                }
            }
            "set" => {
                let key = params
                    .get("key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("missing required parameter: key"))?;
                let value = params
                    .get("value")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("missing required parameter: value"))?;
                self.settings.set(key, value).await?;
                Ok(json!({ "key": key, "updated": true }).into())
            }
            other => Ok(json!({ "error": format!("unknown action: {other}") }).into()),
        }
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
