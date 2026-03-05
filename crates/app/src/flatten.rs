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
///   providers:
///     ollama:
///       base_url: "http://localhost:11434/v1"
///       api_key: "ollama"
///       default_model: "qwen3:32b"
///       fallback_models:
///         - "qwen3:14b"
///         - "llama3:8b"
/// ```
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct LlmConfig {
    pub default_provider: Option<String>,
    pub providers:        HashMap<String, ProviderConfig>,
}

/// Configuration for a single LLM provider (OpenAI-compatible).
///
/// Both fields `base_url` and `api_key` are required at runtime by
/// `OpenAiDriver::resolve_config()`. For local providers like Ollama
/// that don't validate API keys, set `api_key` to any non-empty
/// placeholder (e.g. `"ollama"`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ProviderConfig {
    pub base_url:        Option<String>,
    /// Required for all providers. For Ollama, use any placeholder value (e.g.
    /// `"ollama"`).
    pub api_key:         Option<String>,
    /// Default model for this provider.
    pub default_model:   Option<String>,
    /// Fallback models to try when the default is unavailable.
    pub fallback_models: Option<Vec<String>>,
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
    for (name, provider) in &llm.providers {
        if let Some(ref v) = provider.base_url {
            out.push((format!("llm.providers.{name}.base_url"), v.clone()));
        }
        if let Some(ref v) = provider.api_key {
            out.push((format!("llm.providers.{name}.api_key"), v.clone()));
        }
        if let Some(ref v) = provider.default_model {
            out.push((format!("llm.providers.{name}.default_model"), v.clone()));
        }
        if let Some(ref models) = provider.fallback_models {
            if !models.is_empty() {
                out.push((
                    format!("llm.providers.{name}.fallback_models"),
                    models.join(","),
                ));
            }
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
