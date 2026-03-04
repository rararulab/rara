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

//! Flatten typed config sections into settings key-value pairs.
//!
//! At startup, the values produced here are applied to the settings
//! store, overwriting any existing values.

use std::collections::HashMap;

use serde::Deserialize;

use super::AppConfig;

// ---------------------------------------------------------------------------
// LLM config types
// ---------------------------------------------------------------------------

/// LLM provider configuration section in config.yaml.
///
/// ```yaml
/// llm:
///   default_provider: "ollama"
///   models:
///     default: "qwen3:32b"
///     chat: "qwen3:32b"
///   providers:
///     ollama:
///       base_url: "http://localhost:11434/v1"
///       api_key: "ollama"
/// ```
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct LlmConfig {
    pub default_provider: Option<String>,
    pub models:           LlmModelsConfig,
    pub providers:        HashMap<String, ProviderConfig>,
}

/// Default model configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct LlmModelsConfig {
    pub default: Option<String>,
}

/// Configuration for a single LLM provider (OpenAI-compatible).
///
/// Both fields are required at runtime by `OpenAiDriver::resolve_config()`.
/// For local providers like Ollama that don't validate API keys,
/// set `api_key` to any non-empty placeholder (e.g. `"ollama"`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ProviderConfig {
    pub base_url: Option<String>,
    /// Required for all providers. For Ollama, use any placeholder value (e.g.
    /// `"ollama"`).
    pub api_key:  Option<String>,
}

// ---------------------------------------------------------------------------
// Telegram config types
// ---------------------------------------------------------------------------

/// Telegram bot configuration section in config.yaml.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TelegramConfig {
    pub bot_token:               Option<String>,
    pub chat_id:                 Option<String>,
    pub allowed_group_chat_id:   Option<String>,
    pub notification_channel_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Flatten logic
// ---------------------------------------------------------------------------

/// Flatten all config-file sections into settings key-value pairs.
pub fn flatten_config_sections(config: &AppConfig) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    if let Some(ref llm) = config.llm {
        flatten_llm(llm, &mut pairs);
    }
    if let Some(ref tg) = config.telegram {
        flatten_telegram(tg, &mut pairs);
    }
    pairs
}

fn flatten_llm(llm: &LlmConfig, out: &mut Vec<(String, String)>) {
    if let Some(ref v) = llm.default_provider {
        out.push(("llm.default_provider".into(), v.clone()));
    }
    if let Some(ref v) = llm.models.default {
        out.push(("llm.models.default".into(), v.clone()));
    }
    for (name, provider) in &llm.providers {
        if let Some(ref v) = provider.base_url {
            out.push((format!("llm.providers.{name}.base_url"), v.clone()));
        }
        if let Some(ref v) = provider.api_key {
            out.push((format!("llm.providers.{name}.api_key"), v.clone()));
        }
    }
}

fn flatten_telegram(tg: &TelegramConfig, out: &mut Vec<(String, String)>) {
    if let Some(ref v) = tg.bot_token {
        out.push(("telegram.bot_token".into(), v.clone()));
    }
    if let Some(ref v) = tg.chat_id {
        out.push(("telegram.chat_id".into(), v.clone()));
    }
    if let Some(ref v) = tg.allowed_group_chat_id {
        out.push(("telegram.allowed_group_chat_id".into(), v.clone()));
    }
    if let Some(ref v) = tg.notification_channel_id {
        out.push(("telegram.notification_channel_id".into(), v.clone()));
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flatten_llm_full() {
        let llm = LlmConfig {
            default_provider: Some("ollama".into()),
            models:           LlmModelsConfig {
                default: Some("qwen3:32b".into()),
            },
            providers:        HashMap::from([(
                "ollama".into(),
                ProviderConfig {
                    base_url: Some("http://localhost:11434/v1".into()),
                    api_key:  Some("ollama".into()),
                },
            )]),
        };

        let mut pairs = Vec::new();
        flatten_llm(&llm, &mut pairs);

        let map: HashMap<&str, &str> = pairs
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();

        assert_eq!(map["llm.default_provider"], "ollama");
        assert_eq!(map["llm.models.default"], "qwen3:32b");
        assert_eq!(
            map["llm.providers.ollama.base_url"],
            "http://localhost:11434/v1"
        );
        assert_eq!(map["llm.providers.ollama.api_key"], "ollama");
    }

    #[test]
    fn flatten_telegram_partial() {
        let tg = TelegramConfig {
            bot_token:               Some("123:ABC".into()),
            chat_id:                 Some("456".into()),
            allowed_group_chat_id:   None,
            notification_channel_id: None,
        };

        let mut pairs = Vec::new();
        flatten_telegram(&tg, &mut pairs);

        let map: HashMap<&str, &str> = pairs
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();

        assert_eq!(map["telegram.bot_token"], "123:ABC");
        assert_eq!(map["telegram.chat_id"], "456");
        assert!(!map.contains_key("telegram.allowed_group_chat_id"));
    }

    #[test]
    fn flatten_empty_llm_produces_nothing() {
        let llm = LlmConfig::default();
        let mut pairs = Vec::new();
        flatten_llm(&llm, &mut pairs);
        assert!(pairs.is_empty());
    }
}
