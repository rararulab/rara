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

use serde::{Deserialize, Serialize};

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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_provider: Option<String>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub providers:        HashMap<String, ProviderConfig>,
}

/// Configuration for a single LLM provider (OpenAI-compatible).
///
/// Both fields `base_url` and `api_key` are required at runtime by
/// `OpenAiDriver::resolve_config()`. For local providers like Ollama
/// that don't validate API keys, set `api_key` to any non-empty
/// placeholder (e.g. `"ollama"`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url:        Option<String>,
    /// Required for all providers. For Ollama, use any placeholder value (e.g.
    /// `"ollama"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key:         Option<String>,
    /// Default model for this provider.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_model:   Option<String>,
    /// Fallback models to try when the default is unavailable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_models: Option<Vec<String>>,
}

// ---------------------------------------------------------------------------
// Telegram config types
// ---------------------------------------------------------------------------

/// Telegram bot configuration section in config.yaml.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TelegramConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bot_token:               Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_id:                 Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_group_chat_id:   Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_policy:            Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notification_channel_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Composio config types
// ---------------------------------------------------------------------------

/// Composio configuration section in config.yaml.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ComposioConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key:   Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Knowledge config types
// ---------------------------------------------------------------------------

/// Knowledge layer configuration section in config.yaml.
///
/// ```yaml
/// knowledge:
///   embedding_model: "text-embedding-3-small"
///   embedding_dimensions: 1536
///   search_top_k: 10
///   similarity_threshold: 0.85
///   extractor_model: "gpt-4o-mini"
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct KnowledgeConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_model:      Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_dimensions: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_top_k:         Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub similarity_threshold: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extractor_model:      Option<String>,
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
    if let Some(ref composio) = config.composio {
        flatten_composio(composio, &mut pairs);
    }
    if let Some(ref k) = config.knowledge {
        flatten_knowledge(k, &mut pairs);
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
    if let Some(ref v) = tg.group_policy {
        out.push(("telegram.group_policy".into(), v.clone()));
    }
    if let Some(ref v) = tg.notification_channel_id {
        out.push(("telegram.notification_channel_id".into(), v.clone()));
    }
}

fn flatten_composio(config: &ComposioConfig, out: &mut Vec<(String, String)>) {
    use rara_domain_shared::settings::keys;
    if let Some(ref v) = config.api_key {
        out.push((keys::COMPOSIO_API_KEY.into(), v.clone()));
    }
    if let Some(ref v) = config.entity_id {
        out.push((keys::COMPOSIO_ENTITY_ID.into(), v.clone()));
    }
}

fn flatten_knowledge(k: &KnowledgeConfig, out: &mut Vec<(String, String)>) {
    use rara_domain_shared::settings::keys;
    if let Some(ref v) = k.embedding_model {
        out.push((keys::KNOWLEDGE_EMBEDDING_MODEL.into(), v.clone()));
    }
    if let Some(v) = k.embedding_dimensions {
        out.push((keys::KNOWLEDGE_EMBEDDING_DIMENSIONS.into(), v.to_string()));
    }
    if let Some(v) = k.search_top_k {
        out.push((keys::KNOWLEDGE_SEARCH_TOP_K.into(), v.to_string()));
    }
    if let Some(v) = k.similarity_threshold {
        out.push((keys::KNOWLEDGE_SIMILARITY_THRESHOLD.into(), v.to_string()));
    }
    if let Some(ref v) = k.extractor_model {
        out.push((keys::KNOWLEDGE_EXTRACTOR_MODEL.into(), v.clone()));
    }
}

// ---------------------------------------------------------------------------
// Unflatten logic
// ---------------------------------------------------------------------------

/// Reconstruct config section structs from flat settings KV pairs.
///
/// This is the inverse of [`flatten_config_sections()`]. Keys without
/// a recognised prefix are ignored.
pub fn unflatten_from_settings<S: std::hash::BuildHasher>(
    pairs: &HashMap<String, String, S>,
) -> (
    Option<LlmConfig>,
    Option<TelegramConfig>,
    Option<ComposioConfig>,
    Option<KnowledgeConfig>,
) {
    (
        unflatten_llm(pairs),
        unflatten_telegram(pairs),
        unflatten_composio(pairs),
        unflatten_knowledge(pairs),
    )
}

fn unflatten_llm(pairs: &HashMap<String, String, impl std::hash::BuildHasher>) -> Option<LlmConfig> {
    let mut found = false;
    let mut config = LlmConfig::default();

    if let Some(v) = pairs.get("llm.default_provider") {
        config.default_provider = Some(v.clone());
        found = true;
    }

    // Collect provider names from keys like "llm.providers.{name}.{field}"
    let prefix = "llm.providers.";
    let mut provider_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    for key in pairs.keys() {
        if let Some(rest) = key.strip_prefix(prefix) {
            if let Some(dot_pos) = rest.find('.') {
                provider_names.insert(rest[..dot_pos].to_string());
            }
        }
    }

    for name in &provider_names {
        found = true;
        let p = ProviderConfig {
            base_url:        pairs.get(&format!("{prefix}{name}.base_url")).cloned(),
            api_key:         pairs.get(&format!("{prefix}{name}.api_key")).cloned(),
            default_model:   pairs.get(&format!("{prefix}{name}.default_model")).cloned(),
            fallback_models: pairs
                .get(&format!("{prefix}{name}.fallback_models"))
                .map(|v| v.split(',').map(|s| s.trim().to_string()).collect()),
        };
        config.providers.insert(name.clone(), p);
    }

    found.then_some(config)
}

fn unflatten_telegram(pairs: &HashMap<String, String, impl std::hash::BuildHasher>) -> Option<TelegramConfig> {
    let bot_token = pairs.get("telegram.bot_token").cloned();
    let chat_id = pairs.get("telegram.chat_id").cloned();
    let allowed_group_chat_id = pairs.get("telegram.allowed_group_chat_id").cloned();
    let group_policy = pairs.get("telegram.group_policy").cloned();
    let notification_channel_id = pairs.get("telegram.notification_channel_id").cloned();

    if bot_token.is_none()
        && chat_id.is_none()
        && allowed_group_chat_id.is_none()
        && group_policy.is_none()
        && notification_channel_id.is_none()
    {
        return None;
    }

    Some(TelegramConfig {
        bot_token,
        chat_id,
        allowed_group_chat_id,
        group_policy,
        notification_channel_id,
    })
}

fn unflatten_composio(pairs: &HashMap<String, String, impl std::hash::BuildHasher>) -> Option<ComposioConfig> {
    use rara_domain_shared::settings::keys;
    let api_key = pairs.get(keys::COMPOSIO_API_KEY).cloned();
    let entity_id = pairs.get(keys::COMPOSIO_ENTITY_ID).cloned();

    if api_key.is_none() && entity_id.is_none() {
        return None;
    }

    Some(ComposioConfig { api_key, entity_id })
}

fn unflatten_knowledge(pairs: &HashMap<String, String, impl std::hash::BuildHasher>) -> Option<KnowledgeConfig> {
    use rara_domain_shared::settings::keys;

    let embedding_model = pairs.get(keys::KNOWLEDGE_EMBEDDING_MODEL).cloned();
    let embedding_dimensions = pairs
        .get(keys::KNOWLEDGE_EMBEDDING_DIMENSIONS)
        .and_then(|v| v.parse::<u32>().ok());
    let search_top_k = pairs
        .get(keys::KNOWLEDGE_SEARCH_TOP_K)
        .and_then(|v| v.parse::<u32>().ok());
    let similarity_threshold = pairs
        .get(keys::KNOWLEDGE_SIMILARITY_THRESHOLD)
        .and_then(|v| v.parse::<f32>().ok());
    let extractor_model = pairs.get(keys::KNOWLEDGE_EXTRACTOR_MODEL).cloned();

    if embedding_model.is_none()
        && embedding_dimensions.is_none()
        && search_top_k.is_none()
        && similarity_threshold.is_none()
        && extractor_model.is_none()
    {
        return None;
    }

    Some(KnowledgeConfig {
        embedding_model,
        embedding_dimensions,
        search_top_k,
        similarity_threshold,
        extractor_model,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_flatten_unflatten() {
        let llm = LlmConfig {
            default_provider: Some("ollama".into()),
            providers:        {
                let mut m = HashMap::new();
                m.insert(
                    "ollama".into(),
                    ProviderConfig {
                        base_url:        Some("http://localhost:11434/v1".into()),
                        api_key:         Some("ollama".into()),
                        default_model:   Some("qwen3:32b".into()),
                        fallback_models: Some(vec!["qwen3:14b".into(), "llama3:8b".into()]),
                    },
                );
                m
            },
        };

        let telegram = TelegramConfig {
            bot_token:               Some("123:ABC".into()),
            chat_id:                 Some("456".into()),
            allowed_group_chat_id:   Some("-789".into()),
            group_policy:            Some("mention_or_small_group".into()),
            notification_channel_id: Some("-100".into()),
        };

        let composio = ComposioConfig {
            api_key:   Some("cmp_test_key".into()),
            entity_id: Some("workspace-default".into()),
        };

        let knowledge = KnowledgeConfig {
            embedding_model:      Some("text-embedding-3-small".into()),
            embedding_dimensions: Some(1536),
            search_top_k:         Some(10),
            similarity_threshold: Some(0.85),
            extractor_model:      Some("gpt-4o-mini".into()),
        };

        // Flatten
        let mut flat = Vec::new();
        flatten_llm(&llm, &mut flat);
        flatten_telegram(&telegram, &mut flat);
        flatten_composio(&composio, &mut flat);
        flatten_knowledge(&knowledge, &mut flat);
        let map: HashMap<String, String> = flat.into_iter().collect();

        // Unflatten
        let (got_llm, got_tg, got_composio, got_know) = unflatten_from_settings(&map);

        // --- LLM ---
        let got_llm = got_llm.expect("llm should be Some");
        assert_eq!(got_llm.default_provider, llm.default_provider);
        let got_p = got_llm.providers.get("ollama").expect("ollama provider");
        let exp_p = llm.providers.get("ollama").unwrap();
        assert_eq!(got_p.base_url, exp_p.base_url);
        assert_eq!(got_p.api_key, exp_p.api_key);
        assert_eq!(got_p.default_model, exp_p.default_model);
        assert_eq!(got_p.fallback_models, exp_p.fallback_models);

        // --- Telegram ---
        let got_tg = got_tg.expect("telegram should be Some");
        assert_eq!(got_tg.bot_token, telegram.bot_token);
        assert_eq!(got_tg.chat_id, telegram.chat_id);
        assert_eq!(got_tg.allowed_group_chat_id, telegram.allowed_group_chat_id);
        assert_eq!(got_tg.group_policy, telegram.group_policy);
        assert_eq!(
            got_tg.notification_channel_id,
            telegram.notification_channel_id
        );

        // --- Composio ---
        let got_composio = got_composio.expect("composio should be Some");
        assert_eq!(got_composio.api_key, composio.api_key);
        assert_eq!(got_composio.entity_id, composio.entity_id);

        // --- Knowledge ---
        let got_know = got_know.expect("knowledge should be Some");
        assert_eq!(got_know.embedding_model, knowledge.embedding_model);
        assert_eq!(
            got_know.embedding_dimensions,
            knowledge.embedding_dimensions
        );
        assert_eq!(got_know.search_top_k, knowledge.search_top_k);
        assert_eq!(
            got_know.similarity_threshold,
            knowledge.similarity_threshold
        );
        assert_eq!(got_know.extractor_model, knowledge.extractor_model);
    }

    #[test]
    fn unflatten_empty_map_returns_none() {
        let map = HashMap::new();
        let (llm, tg, composio, know) = unflatten_from_settings(&map);
        assert!(llm.is_none());
        assert!(tg.is_none());
        assert!(composio.is_none());
        assert!(know.is_none());
    }
}
