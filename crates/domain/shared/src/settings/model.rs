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

//! Runtime-configurable application settings shared across services.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Full runtime settings document persisted in `kv_table`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Settings {
    pub ai:           AISettings,
    pub telegram:     TelegramSettings,
    #[serde(default)]
    pub agent:        AgentSettings,
    #[serde(default)]
    pub job_pipeline: JobPipelineSettings,
    #[serde(default)]
    pub workers:      WorkerSettings,
    pub updated_at:   Option<chrono::DateTime<chrono::Utc>>,
}

/// AI-specific runtime settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AISettings {
    pub openrouter_api_key: Option<String>,
    /// LLM provider: `"openrouter"` (default) or `"ollama"`.
    #[serde(default)]
    pub provider:           Option<String>,
    /// Ollama API base URL. Defaults to `http://localhost:11434`.
    #[serde(default)]
    pub ollama_base_url:    Option<String>,
    /// Key-based model assignments (e.g. `"chat" -> "openai/gpt-4o"`).
    /// Well-known keys: `default`, `chat`, `job`, `pipeline`, `proactive`,
    /// `scheduled`.
    #[serde(default)]
    pub models:             HashMap<String, String>,
    /// Global fallback model list, tried in order when the primary fails.
    #[serde(default)]
    pub fallback_models:    Vec<String>,
    /// User-pinned model IDs shown at the top of the model picker.
    #[serde(default)]
    pub favorite_models:    Vec<String>,
}

impl AISettings {
    /// Whether any LLM provider is configured.
    ///
    /// Ollama does not require an API key, so it is always considered
    /// configured. OpenRouter requires `openrouter_api_key` to be set.
    pub fn is_configured(&self) -> bool {
        match self.provider.as_deref().unwrap_or("openrouter") {
            "ollama" => true, // Ollama doesn't need an API key
            _ => self.openrouter_api_key.is_some(),
        }
    }

    /// Resolve the model for the given key.
    ///
    /// Falls back to the `"default"` key, then to the hardcoded default.
    pub fn model_for_key(&self, key: &str) -> String {
        self.models
            .get(key)
            .or_else(|| self.models.get("default"))
            .cloned()
            .unwrap_or_else(|| "openai/gpt-4o".to_owned())
    }
}

/// Telegram-specific runtime settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TelegramSettings {
    pub bot_token:               Option<String>,
    pub chat_id:                 Option<i64>,
    pub allowed_group_chat_id:   Option<i64>,
    /// Dedicated Telegram channel/group ID for automated notifications
    /// (e.g. pipeline results). When set, pipeline notifications are sent
    /// directly via the Bot API instead of going through PGMQ.
    #[serde(default)]
    pub notification_channel_id: Option<i64>,
}

/// Agent personality and proactive messaging settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AgentSettings {
    /// Whether proactive messaging is enabled.
    pub proactive_enabled:  bool,
    /// Cron expression for proactive check schedule (5-field format).
    /// Changes take effect after service restart.
    pub proactive_cron:     Option<String>,
    /// Maximum number of tool-call loop iterations for agent runs.
    /// `None` uses the compile-time default (25).
    pub max_iterations:     Option<u32>,
    /// Memory retrieval runtime configuration.
    #[serde(default)]
    pub memory:             MemorySettings,
    /// Composio tool runtime authentication settings.
    #[serde(default)]
    pub composio:           ComposioSettings,
    /// Gmail SMTP settings for email sending.
    #[serde(default)]
    pub gmail:              GmailSettings,
}

/// Memory runtime settings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct MemorySettings {
    /// Chroma server base URL.
    pub chroma_url:        Option<String>,
    /// Chroma collection name.
    pub chroma_collection: Option<String>,
    /// Chroma API key/token.
    pub chroma_api_key:    Option<String>,
}

impl Default for MemorySettings {
    fn default() -> Self {
        Self {
            chroma_url:        Some("http://localhost:8000".to_owned()),
            chroma_collection: Some("job-memory".to_owned()),
            chroma_api_key:    None,
        }
    }
}

/// Composio runtime settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ComposioSettings {
    /// Composio API key used by the composio primitive.
    pub api_key:   Option<String>,
    /// Optional default user/entity id for composio calls.
    pub entity_id: Option<String>,
}

/// Job pipeline runtime settings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct JobPipelineSettings {
    /// Markdown describing target roles, tech stack, preferences.
    pub job_preferences:        Option<String>,
    /// Auto-apply score threshold (default 85).
    pub score_threshold_auto:   u8,
    /// Notification score threshold (default 60).
    pub score_threshold_notify: u8,
    /// Local path to typst resume project.
    pub resume_project_path:    Option<String>,
    /// Cron expression for automatic pipeline runs (5-field format). `None` =
    /// disabled.
    pub pipeline_cron:          Option<String>,
}

impl Default for JobPipelineSettings {
    fn default() -> Self {
        Self {
            job_preferences:        None,
            score_threshold_auto:   85,
            score_threshold_notify: 60,
            resume_project_path:    None,
            pipeline_cron:          None,
        }
    }
}

/// Gmail SMTP runtime settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct GmailSettings {
    /// Gmail address used as the sender (e.g. `user@gmail.com`).
    pub address:           Option<String>,
    /// Gmail App Password for SMTP authentication.
    pub app_password:      Option<String>,
    /// Whether the agent is allowed to send emails automatically.
    pub auto_send_enabled: bool,
}

/// Worker poll interval settings.
///
/// These control how often background workers wake up and check for work.
/// Changes take effect after service restart.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct WorkerSettings {
    /// Agent scheduler poll interval in seconds (default 60).
    pub agent_scheduler_interval_secs:  u64,
    /// Pipeline scheduler poll interval in seconds (default 60).
    pub pipeline_scheduler_interval_secs: u64,
    /// Memory sync interval in seconds (default 300 = 5 min).
    pub memory_sync_interval_secs:      u64,
    /// Proactive agent interval in hours (default 12).
    pub proactive_agent_interval_hours: u64,
}

impl Default for WorkerSettings {
    fn default() -> Self {
        Self {
            agent_scheduler_interval_secs:    60,
            pipeline_scheduler_interval_secs: 60,
            memory_sync_interval_secs:        300,
            proactive_agent_interval_hours:   12,
        }
    }
}

/// Partial update payload for runtime settings writes.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct UpdateRequest {
    pub ai:           Option<AiRuntimeSettingsPatch>,
    pub telegram:     Option<TelegramRuntimeSettingsPatch>,
    pub agent:        Option<AgentRuntimeSettingsPatch>,
    pub job_pipeline: Option<JobPipelineRuntimeSettingsPatch>,
    pub workers:      Option<WorkerRuntimeSettingsPatch>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct AiRuntimeSettingsPatch {
    pub openrouter_api_key: Option<String>,
    /// LLM provider: `"openrouter"` or `"ollama"`. Empty string clears
    /// (reverts to default `"openrouter"`).
    pub provider:           Option<String>,
    /// Ollama API base URL. Empty string clears (reverts to default).
    pub ollama_base_url:    Option<String>,
    /// Key-based model patches. `Some(model)` sets the key,
    /// `None` (or empty string) removes the key.
    #[schema(value_type = Option<HashMap<String, Option<String>>>)]
    pub models:             Option<HashMap<String, Option<String>>>,
    /// Replace the global fallback models list. `None` to leave unchanged.
    pub fallback_models:    Option<Vec<String>>,
    /// Replace the entire favorite models list. `None` to leave unchanged.
    pub favorite_models:    Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct TelegramRuntimeSettingsPatch {
    pub bot_token:               Option<String>,
    pub chat_id:                 Option<i64>,
    pub allowed_group_chat_id:   Option<i64>,
    /// Double-Option: `None` = leave unchanged, `Some(None)` = clear,
    /// `Some(Some(id))` = set.
    pub notification_channel_id: Option<Option<i64>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct AgentRuntimeSettingsPatch {
    pub proactive_enabled:  Option<bool>,
    pub proactive_cron:     Option<String>,
    /// Set the max iterations limit. `Some(0)` or `None` leaves it unchanged.
    /// Use `Some(n)` where `n > 0` to override.
    pub max_iterations:     Option<u32>,
    pub memory:             Option<MemoryRuntimeSettingsPatch>,
    pub composio:           Option<ComposioRuntimeSettingsPatch>,
    pub gmail:              Option<GmailRuntimeSettingsPatch>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct MemoryRuntimeSettingsPatch {
    pub chroma_url:        Option<String>,
    pub chroma_collection: Option<String>,
    pub chroma_api_key:    Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct ComposioRuntimeSettingsPatch {
    pub api_key:   Option<String>,
    pub entity_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct JobPipelineRuntimeSettingsPatch {
    pub job_preferences:        Option<String>,
    pub score_threshold_auto:   Option<u8>,
    pub score_threshold_notify: Option<u8>,
    pub resume_project_path:    Option<String>,
    pub pipeline_cron:          Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct GmailRuntimeSettingsPatch {
    pub address:           Option<String>,
    pub app_password:      Option<String>,
    pub auto_send_enabled: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct WorkerRuntimeSettingsPatch {
    pub agent_scheduler_interval_secs:    Option<u64>,
    pub pipeline_scheduler_interval_secs: Option<u64>,
    pub memory_sync_interval_secs:        Option<u64>,
    pub proactive_agent_interval_hours:   Option<u64>,
}

impl Settings {
    /// Apply a partial update patch.
    pub fn apply_patch(&mut self, patch: UpdateRequest) {
        if let Some(ai) = patch.ai {
            if let Some(key) = ai.openrouter_api_key {
                self.ai.openrouter_api_key = normalize_secret(Some(key));
            }
            if let Some(provider) = ai.provider {
                self.ai.provider = normalize_text(Some(provider));
            }
            if let Some(url) = ai.ollama_base_url {
                self.ai.ollama_base_url = normalize_text(Some(url));
            }
            if let Some(models_patch) = ai.models {
                for (key, value) in models_patch {
                    match value {
                        Some(model) if !model.trim().is_empty() => {
                            self.ai.models.insert(key, model);
                        }
                        _ => {
                            self.ai.models.remove(&key);
                        }
                    }
                }
            }
            if let Some(fallbacks) = ai.fallback_models {
                self.ai.fallback_models = fallbacks;
            }
            if let Some(favorites) = ai.favorite_models {
                self.ai.favorite_models = favorites;
            }
        }

        if let Some(telegram) = patch.telegram {
            if let Some(token) = telegram.bot_token {
                self.telegram.bot_token = normalize_secret(Some(token));
            }
            if let Some(chat_id) = telegram.chat_id {
                self.telegram.chat_id = Some(chat_id);
            }
            if let Some(allowed_group_chat_id) = telegram.allowed_group_chat_id {
                self.telegram.allowed_group_chat_id = Some(allowed_group_chat_id);
            }
            if let Some(notification_channel_id) = telegram.notification_channel_id {
                self.telegram.notification_channel_id = notification_channel_id;
            }
        }

        if let Some(agent) = patch.agent {
            if let Some(enabled) = agent.proactive_enabled {
                self.agent.proactive_enabled = enabled;
            }
            if let Some(cron) = agent.proactive_cron {
                self.agent.proactive_cron = normalize_text(Some(cron));
            }
            if let Some(max_iter) = agent.max_iterations {
                // 0 clears the override (reverts to code default).
                self.agent.max_iterations = if max_iter == 0 { None } else { Some(max_iter) };
            }
            if let Some(memory) = agent.memory {
                if let Some(chroma_url) = memory.chroma_url {
                    self.agent.memory.chroma_url = normalize_text(Some(chroma_url));
                }
                if let Some(chroma_collection) = memory.chroma_collection {
                    self.agent.memory.chroma_collection = normalize_text(Some(chroma_collection));
                }
                if let Some(chroma_api_key) = memory.chroma_api_key {
                    self.agent.memory.chroma_api_key = normalize_secret(Some(chroma_api_key));
                }
            }
            if let Some(composio) = agent.composio {
                if let Some(api_key) = composio.api_key {
                    self.agent.composio.api_key = normalize_secret(Some(api_key));
                }
                if let Some(entity_id) = composio.entity_id {
                    self.agent.composio.entity_id = normalize_text(Some(entity_id));
                }
            }
            if let Some(gmail) = agent.gmail {
                if let Some(address) = gmail.address {
                    self.agent.gmail.address = normalize_text(Some(address));
                }
                if let Some(app_password) = gmail.app_password {
                    self.agent.gmail.app_password = normalize_secret(Some(app_password));
                }
                if let Some(auto_send_enabled) = gmail.auto_send_enabled {
                    self.agent.gmail.auto_send_enabled = auto_send_enabled;
                }
            }
        }

        if let Some(jp) = patch.job_pipeline {
            if let Some(prefs) = jp.job_preferences {
                self.job_pipeline.job_preferences = normalize_text(Some(prefs));
            }
            if let Some(threshold) = jp.score_threshold_auto {
                self.job_pipeline.score_threshold_auto = threshold;
            }
            if let Some(threshold) = jp.score_threshold_notify {
                self.job_pipeline.score_threshold_notify = threshold;
            }
            if let Some(path) = jp.resume_project_path {
                self.job_pipeline.resume_project_path = normalize_text(Some(path));
            }
            if let Some(cron) = jp.pipeline_cron {
                self.job_pipeline.pipeline_cron = normalize_text(Some(cron));
            }
        }

        if let Some(w) = patch.workers {
            if let Some(v) = w.agent_scheduler_interval_secs {
                self.workers.agent_scheduler_interval_secs = v;
            }
            if let Some(v) = w.pipeline_scheduler_interval_secs {
                self.workers.pipeline_scheduler_interval_secs = v;
            }
            if let Some(v) = w.memory_sync_interval_secs {
                self.workers.memory_sync_interval_secs = v;
            }
            if let Some(v) = w.proactive_agent_interval_hours {
                self.workers.proactive_agent_interval_hours = v;
            }
        }
    }

    /// Sanitize values by trimming and dropping empty strings.
    pub fn normalize(&mut self) {
        self.ai.openrouter_api_key = normalize_secret(self.ai.openrouter_api_key.take());
        self.ai.provider = normalize_text(self.ai.provider.take());
        self.ai.ollama_base_url = normalize_text(self.ai.ollama_base_url.take());
        // Normalize models map: trim values, remove entries with empty values.
        self.ai.models = std::mem::take(&mut self.ai.models)
            .into_iter()
            .filter_map(|(k, v)| {
                let trimmed = v.trim().to_owned();
                if trimmed.is_empty() {
                    None
                } else {
                    Some((k, trimmed))
                }
            })
            .collect();
        normalize_string_list(&mut self.ai.fallback_models);
        self.ai.favorite_models.retain(|s| !s.trim().is_empty());
        self.ai.favorite_models.dedup();
        self.telegram.bot_token = normalize_secret(self.telegram.bot_token.take());
        self.agent.proactive_cron = normalize_text(self.agent.proactive_cron.take());
        self.agent.memory.chroma_url = normalize_text(self.agent.memory.chroma_url.take());
        self.agent.memory.chroma_collection =
            normalize_text(self.agent.memory.chroma_collection.take());
        self.agent.memory.chroma_api_key =
            normalize_secret(self.agent.memory.chroma_api_key.take());
        self.agent.composio.api_key = normalize_secret(self.agent.composio.api_key.take());
        self.agent.composio.entity_id = normalize_text(self.agent.composio.entity_id.take());
        self.agent.gmail.address = normalize_text(self.agent.gmail.address.take());
        self.agent.gmail.app_password = normalize_secret(self.agent.gmail.app_password.take());

        self.job_pipeline.job_preferences =
            normalize_text(self.job_pipeline.job_preferences.take());
        self.job_pipeline.resume_project_path =
            normalize_text(self.job_pipeline.resume_project_path.take());
        self.job_pipeline.pipeline_cron = normalize_text(self.job_pipeline.pipeline_cron.take());
    }
}

fn normalize_text(value: Option<String>) -> Option<String> {
    value.and_then(|v| {
        let trimmed = v.trim().to_owned();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

fn normalize_secret(value: Option<String>) -> Option<String> { normalize_text(value) }

/// Trim each entry, drop empty strings, and deduplicate.
fn normalize_string_list(list: &mut Vec<String>) {
    for item in list.iter_mut() {
        *item = item.trim().to_owned();
    }
    list.retain(|s| !s.is_empty());
    list.dedup();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_for_key_falls_back_to_hardcoded_default() {
        let ai = AISettings::default();
        assert_eq!(ai.model_for_key("job"), "openai/gpt-4o");
        assert_eq!(ai.model_for_key("chat"), "openai/gpt-4o");
    }

    #[test]
    fn model_for_key_uses_default_key_when_no_specific() {
        let ai = AISettings {
            models: HashMap::from([("default".to_owned(), "anthropic/claude-sonnet-4".to_owned())]),
            ..Default::default()
        };
        assert_eq!(ai.model_for_key("job"), "anthropic/claude-sonnet-4");
        assert_eq!(ai.model_for_key("chat"), "anthropic/claude-sonnet-4");
    }

    #[test]
    fn model_for_key_uses_specific_key() {
        let ai = AISettings {
            models: HashMap::from([
                ("default".to_owned(), "anthropic/claude-sonnet-4".to_owned()),
                ("job".to_owned(), "openai/gpt-4o".to_owned()),
                ("chat".to_owned(), "openai/gpt-4o-mini".to_owned()),
            ]),
            ..Default::default()
        };
        assert_eq!(ai.model_for_key("job"), "openai/gpt-4o");
        assert_eq!(ai.model_for_key("chat"), "openai/gpt-4o-mini");
    }

    #[test]
    fn model_for_key_partial_override() {
        let ai = AISettings {
            models: HashMap::from([
                ("default".to_owned(), "anthropic/claude-sonnet-4".to_owned()),
                ("job".to_owned(), "openai/gpt-4o".to_owned()),
            ]),
            ..Default::default()
        };
        assert_eq!(ai.model_for_key("job"), "openai/gpt-4o");
        // Chat falls back to default key
        assert_eq!(ai.model_for_key("chat"), "anthropic/claude-sonnet-4");
    }

    #[test]
    fn apply_patch_sets_and_removes_models() {
        let mut settings = Settings {
            ai: AISettings {
                models: HashMap::from([("job".to_owned(), "openai/gpt-4o".to_owned())]),
                ..Default::default()
            },
            ..Default::default()
        };
        // Set chat, remove job (via None)
        settings.apply_patch(UpdateRequest {
            ai:           Some(AiRuntimeSettingsPatch {
                models: Some(HashMap::from([
                    ("chat".to_owned(), Some("anthropic/claude-sonnet-4".to_owned())),
                    ("job".to_owned(), None), // remove
                ])),
                ..Default::default()
            }),
            telegram:     None,
            agent:        None,
            job_pipeline: None,
            workers:      None,
        });
        assert_eq!(
            settings.ai.models.get("chat").map(String::as_str),
            Some("anthropic/claude-sonnet-4")
        );
        assert_eq!(settings.ai.models.get("job"), None);
    }

    #[test]
    fn apply_patch_clears_model_with_empty_string() {
        let mut settings = Settings {
            ai: AISettings {
                models: HashMap::from([("job".to_owned(), "openai/gpt-4o".to_owned())]),
                ..Default::default()
            },
            ..Default::default()
        };
        settings.apply_patch(UpdateRequest {
            ai:           Some(AiRuntimeSettingsPatch {
                models: Some(HashMap::from([
                    ("job".to_owned(), Some("".to_owned())), // empty string clears
                ])),
                ..Default::default()
            }),
            telegram:     None,
            agent:        None,
            job_pipeline: None,
            workers:      None,
        });
        assert_eq!(settings.ai.models.get("job"), None);
    }

    #[test]
    fn normalize_clears_whitespace_only_models() {
        let mut settings = Settings {
            ai: AISettings {
                models: HashMap::from([
                    ("default".to_owned(), "  ".to_owned()),
                    ("job".to_owned(), "  openai/gpt-4o  ".to_owned()),
                    ("chat".to_owned(), "".to_owned()),
                ]),
                ..Default::default()
            },
            ..Default::default()
        };
        settings.normalize();
        assert_eq!(settings.ai.models.get("default"), None);
        assert_eq!(
            settings.ai.models.get("job").map(String::as_str),
            Some("openai/gpt-4o")
        );
        assert_eq!(settings.ai.models.get("chat"), None);
    }

    #[test]
    fn apply_patch_fallback_models() {
        let mut settings = Settings::default();
        settings.apply_patch(UpdateRequest {
            ai:           Some(AiRuntimeSettingsPatch {
                fallback_models: Some(vec![
                    "anthropic/claude-sonnet-4".to_owned(),
                    "google/gemini-2.0-flash".to_owned(),
                ]),
                ..Default::default()
            }),
            telegram:     None,
            agent:        None,
            job_pipeline: None,
            workers:      None,
        });
        assert_eq!(
            settings.ai.fallback_models,
            vec!["anthropic/claude-sonnet-4", "google/gemini-2.0-flash"]
        );
    }

    #[test]
    fn normalize_trims_and_deduplicates_fallbacks() {
        let mut settings = Settings {
            ai: AISettings {
                fallback_models: vec![
                    "  openai/gpt-4o  ".to_owned(),
                    "".to_owned(),
                    "openai/gpt-4o".to_owned(), // dup after trim
                    "anthropic/claude-sonnet-4".to_owned(),
                ],
                ..Default::default()
            },
            ..Default::default()
        };
        settings.normalize();
        assert_eq!(
            settings.ai.fallback_models,
            vec!["openai/gpt-4o", "anthropic/claude-sonnet-4"]
        );
    }

    #[test]
    fn agent_settings_default_values() {
        let settings = Settings::default();
        assert!(!settings.agent.proactive_enabled);
        assert_eq!(settings.agent.proactive_cron, None);
    }

    #[test]
    fn apply_patch_agent_settings() {
        let mut settings = Settings::default();
        settings.apply_patch(UpdateRequest {
            ai:           None,
            telegram:     None,
            agent:        Some(AgentRuntimeSettingsPatch {
                proactive_enabled:  Some(true),
                proactive_cron:     Some("0 9 * * *".to_owned()),
                memory:             None,
                composio:           None,
                gmail:              None,
                max_iterations:     None,
            }),
            job_pipeline: None,
            workers:      None,
        });
        assert!(settings.agent.proactive_enabled);
        assert_eq!(settings.agent.proactive_cron, Some("0 9 * * *".to_owned()));
    }

    #[test]
    fn apply_patch_agent_partial() {
        let mut settings = Settings {
            agent: AgentSettings {
                proactive_enabled:  true,
                proactive_cron:     Some("0 9 * * *".to_owned()),
                memory:             MemorySettings::default(),
                composio:           ComposioSettings::default(),
                gmail:              GmailSettings::default(),
                max_iterations:     None,
            },
            ..Default::default()
        };
        settings.apply_patch(UpdateRequest {
            ai:           None,
            telegram:     None,
            agent:        Some(AgentRuntimeSettingsPatch {
                proactive_enabled:  Some(false),
                proactive_cron:     None,
                memory:             None,
                composio:           None,
                gmail:              None,
                max_iterations:     None,
            }),
            job_pipeline: None,
            workers:      None,
        });
        assert!(!settings.agent.proactive_enabled);
        assert_eq!(settings.agent.proactive_cron, Some("0 9 * * *".to_owned()));
    }

    #[test]
    fn normalize_agent_settings() {
        let mut settings = Settings {
            agent: AgentSettings {
                proactive_enabled:  true,
                proactive_cron:     Some("  0 9 * * *  ".to_owned()),
                memory:             MemorySettings::default(),
                composio:           ComposioSettings::default(),
                gmail:              GmailSettings::default(),
                max_iterations:     None,
            },
            ..Default::default()
        };
        settings.normalize();
        assert_eq!(settings.agent.proactive_cron, Some("0 9 * * *".to_owned()));
    }

    #[test]
    fn agent_settings_serde_default() {
        let json = r#"{"ai":{},"telegram":{}}"#;
        let settings: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(settings.agent, AgentSettings::default());
    }

    #[test]
    fn apply_patch_memory_settings() {
        let mut settings = Settings::default();
        settings.apply_patch(UpdateRequest {
            ai:           None,
            telegram:     None,
            agent:        Some(AgentRuntimeSettingsPatch {
                proactive_enabled:  None,
                proactive_cron:     None,
                memory:             Some(MemoryRuntimeSettingsPatch {
                    chroma_url:        Some("http://localhost:8000".to_owned()),
                    chroma_collection: Some("team-memory".to_owned()),
                    chroma_api_key:    Some("secret-token".to_owned()),
                }),
                composio:           None,
                gmail:              None,
                max_iterations:     None,
            }),
            job_pipeline: None,
            workers:      None,
        });

        assert_eq!(
            settings.agent.memory.chroma_url,
            Some("http://localhost:8000".to_owned())
        );
        assert_eq!(
            settings.agent.memory.chroma_collection,
            Some("team-memory".to_owned())
        );
        assert_eq!(
            settings.agent.memory.chroma_api_key,
            Some("secret-token".to_owned())
        );
    }

    // -- job_pipeline tests ---------------------------------------------------

    #[test]
    fn job_pipeline_default_values() {
        let settings = Settings::default();
        assert_eq!(settings.job_pipeline.job_preferences, None);
        assert_eq!(settings.job_pipeline.score_threshold_auto, 85);
        assert_eq!(settings.job_pipeline.score_threshold_notify, 60);
        assert_eq!(settings.job_pipeline.resume_project_path, None);
    }

    #[test]
    fn apply_patch_job_pipeline_settings() {
        let mut settings = Settings::default();
        settings.apply_patch(UpdateRequest {
            ai:           None,
            telegram:     None,
            agent:        None,
            job_pipeline: Some(JobPipelineRuntimeSettingsPatch {
                job_preferences:        Some("Rust backend, distributed systems".to_owned()),
                score_threshold_auto:   Some(90),
                score_threshold_notify: Some(70),
                resume_project_path:    Some("/home/user/resume".to_owned()),
                pipeline_cron:          None,
            }),
            workers:      None,
        });
        assert_eq!(
            settings.job_pipeline.job_preferences,
            Some("Rust backend, distributed systems".to_owned())
        );
        assert_eq!(settings.job_pipeline.score_threshold_auto, 90);
        assert_eq!(settings.job_pipeline.score_threshold_notify, 70);
        assert_eq!(
            settings.job_pipeline.resume_project_path,
            Some("/home/user/resume".to_owned())
        );
    }

    #[test]
    fn apply_patch_job_pipeline_partial() {
        let mut settings = Settings::default();
        settings.apply_patch(UpdateRequest {
            ai:           None,
            telegram:     None,
            agent:        None,
            job_pipeline: Some(JobPipelineRuntimeSettingsPatch {
                job_preferences:        None,
                score_threshold_auto:   Some(95),
                score_threshold_notify: None,
                resume_project_path:    None,
                pipeline_cron:          None,
            }),
            workers:      None,
        });
        assert_eq!(settings.job_pipeline.score_threshold_auto, 95);
        assert_eq!(settings.job_pipeline.score_threshold_notify, 60);
    }

    #[test]
    fn normalize_job_pipeline_settings() {
        let mut settings = Settings {
            job_pipeline: JobPipelineSettings {
                job_preferences:        Some("  ".to_owned()),
                score_threshold_auto:   85,
                score_threshold_notify: 60,
                resume_project_path:    Some("  /home/user/resume  ".to_owned()),
                pipeline_cron:          None,
            },
            ..Default::default()
        };
        settings.normalize();
        assert_eq!(settings.job_pipeline.job_preferences, None);
        assert_eq!(
            settings.job_pipeline.resume_project_path,
            Some("/home/user/resume".to_owned())
        );
    }

    // -- gmail tests ----------------------------------------------------------

    #[test]
    fn gmail_default_values() {
        let settings = Settings::default();
        assert_eq!(settings.agent.gmail.address, None);
        assert_eq!(settings.agent.gmail.app_password, None);
        assert!(!settings.agent.gmail.auto_send_enabled);
    }

    #[test]
    fn apply_patch_gmail_settings() {
        let mut settings = Settings::default();
        settings.apply_patch(UpdateRequest {
            ai:           None,
            telegram:     None,
            agent:        Some(AgentRuntimeSettingsPatch {
                proactive_enabled:  None,
                proactive_cron:     None,
                memory:             None,
                composio:           None,
                gmail:              Some(GmailRuntimeSettingsPatch {
                    address:           Some("user@gmail.com".to_owned()),
                    app_password:      Some("abcd-efgh-ijkl-mnop".to_owned()),
                    auto_send_enabled: Some(true),
                }),
                max_iterations:     None,
            }),
            job_pipeline: None,
            workers:      None,
        });
        assert_eq!(
            settings.agent.gmail.address,
            Some("user@gmail.com".to_owned())
        );
        assert_eq!(
            settings.agent.gmail.app_password,
            Some("abcd-efgh-ijkl-mnop".to_owned())
        );
        assert!(settings.agent.gmail.auto_send_enabled);
    }

    #[test]
    fn normalize_gmail_settings() {
        let mut settings = Settings::default();
        settings.agent.gmail.address = Some("  user@gmail.com  ".to_owned());
        settings.agent.gmail.app_password = Some("  ".to_owned());
        settings.normalize();
        assert_eq!(
            settings.agent.gmail.address,
            Some("user@gmail.com".to_owned())
        );
        assert_eq!(settings.agent.gmail.app_password, None);
    }

    #[test]
    fn serde_default_backward_compat() {
        let json = r#"{"ai":{},"telegram":{}}"#;
        let settings: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(settings.job_pipeline, JobPipelineSettings::default());
        assert_eq!(settings.agent.gmail, GmailSettings::default());
        assert_eq!(settings.workers, WorkerSettings::default());
    }

    // -- worker settings tests ------------------------------------------------

    #[test]
    fn worker_settings_default_values() {
        let settings = Settings::default();
        assert_eq!(settings.workers.agent_scheduler_interval_secs, 60);
        assert_eq!(settings.workers.pipeline_scheduler_interval_secs, 60);
        assert_eq!(settings.workers.memory_sync_interval_secs, 300);
        assert_eq!(settings.workers.proactive_agent_interval_hours, 12);
    }

    #[test]
    fn apply_patch_worker_settings() {
        let mut settings = Settings::default();
        settings.apply_patch(UpdateRequest {
            ai:           None,
            telegram:     None,
            agent:        None,
            job_pipeline: None,
            workers:      Some(WorkerRuntimeSettingsPatch {
                agent_scheduler_interval_secs:    Some(120),
                pipeline_scheduler_interval_secs: Some(90),
                memory_sync_interval_secs:        Some(600),
                proactive_agent_interval_hours:   Some(24),
            }),
        });
        assert_eq!(settings.workers.agent_scheduler_interval_secs, 120);
        assert_eq!(settings.workers.pipeline_scheduler_interval_secs, 90);
        assert_eq!(settings.workers.memory_sync_interval_secs, 600);
        assert_eq!(settings.workers.proactive_agent_interval_hours, 24);
    }

    #[test]
    fn apply_patch_worker_settings_partial() {
        let mut settings = Settings::default();
        settings.apply_patch(UpdateRequest {
            ai:           None,
            telegram:     None,
            agent:        None,
            job_pipeline: None,
            workers:      Some(WorkerRuntimeSettingsPatch {
                agent_scheduler_interval_secs:    Some(120),
                pipeline_scheduler_interval_secs: None,
                memory_sync_interval_secs:        None,
                proactive_agent_interval_hours:   None,
            }),
        });
        assert_eq!(settings.workers.agent_scheduler_interval_secs, 120);
        // Unchanged fields retain defaults.
        assert_eq!(settings.workers.pipeline_scheduler_interval_secs, 60);
        assert_eq!(settings.workers.memory_sync_interval_secs, 300);
        assert_eq!(settings.workers.proactive_agent_interval_hours, 12);
    }
}
