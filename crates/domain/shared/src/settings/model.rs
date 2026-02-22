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
    pub updated_at:   Option<chrono::DateTime<chrono::Utc>>,
}

// TODO: optimize it, we dont need to hardcode it.
/// Which scenario an AI model will be used for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelScenario {
    /// Job analysis tasks (fit scoring, JD parsing, resume optimization, etc.)
    Job,
    /// Interactive chat conversations
    Chat,
}

/// AI-specific runtime settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AISettings {
    pub openrouter_api_key:   Option<String>,
    pub default_model:        Option<String>,
    pub job_model:            Option<String>,
    pub chat_model:           Option<String>,
    /// User-pinned model IDs shown at the top of the model picker.
    #[serde(default)]
    pub favorite_models:      Vec<String>,
    /// Fallback models for chat scenario, tried in order when primary fails.
    #[serde(default)]
    pub chat_model_fallbacks: Vec<String>,
    /// Fallback models for job scenario, tried in order when primary fails.
    #[serde(default)]
    pub job_model_fallbacks:  Vec<String>,
}

impl AISettings {
    /// Resolve the model identifier for a given scenario.
    ///
    /// Falls back to `default_model`, then to `"openai/gpt-4o"`.
    pub fn model_for(&self, scenario: ModelScenario) -> &str {
        let specific = match scenario {
            ModelScenario::Job => self.job_model.as_deref(),
            ModelScenario::Chat => self.chat_model.as_deref(),
        };
        specific
            .or(self.default_model.as_deref())
            .unwrap_or("openai/gpt-4o")
    }

    /// Return the ordered fallback chain for a given scenario.
    ///
    /// The chain starts with the primary model (`model_for(scenario)`) and
    /// appends any configured fallback models, skipping empty strings and
    /// duplicates of the primary.
    pub fn fallback_chain(&self, scenario: ModelScenario) -> Vec<&str> {
        let primary = self.model_for(scenario);
        let fallbacks = match scenario {
            ModelScenario::Job => &self.job_model_fallbacks,
            ModelScenario::Chat => &self.chat_model_fallbacks,
        };
        let mut chain = vec![primary];
        for fb in fallbacks {
            let fb = fb.trim();
            if !fb.is_empty() && fb != primary {
                chain.push(fb);
            }
        }
        chain
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
    /// The agent's personality/soul prompt. `None` uses the built-in default.
    pub soul:               Option<String>,
    /// Custom system prompt for chat sessions. `None` uses the built-in
    /// default.
    pub chat_system_prompt: Option<String>,
    /// Whether proactive messaging is enabled.
    pub proactive_enabled:  bool,
    /// Cron expression for proactive check schedule (5-field format).
    /// Changes take effect after service restart.
    pub proactive_cron:     Option<String>,
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

/// Partial update payload for runtime settings writes.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct UpdateRequest {
    pub ai:           Option<AiRuntimeSettingsPatch>,
    pub telegram:     Option<TelegramRuntimeSettingsPatch>,
    pub agent:        Option<AgentRuntimeSettingsPatch>,
    pub job_pipeline: Option<JobPipelineRuntimeSettingsPatch>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct AiRuntimeSettingsPatch {
    pub openrouter_api_key:   Option<String>,
    pub default_model:        Option<String>,
    /// `Some(model)` to set, `None` to leave unchanged.
    /// Use `Some("")` or send an empty string to clear (revert to default).
    pub job_model:            Option<String>,
    /// `Some(model)` to set, `None` to leave unchanged.
    /// Use `Some("")` or send an empty string to clear (revert to default).
    pub chat_model:           Option<String>,
    /// Replace the entire favorite models list. `None` to leave unchanged.
    pub favorite_models:      Option<Vec<String>>,
    /// Replace the chat fallback models list. `None` to leave unchanged.
    pub chat_model_fallbacks: Option<Vec<String>>,
    /// Replace the job fallback models list. `None` to leave unchanged.
    pub job_model_fallbacks:  Option<Vec<String>>,
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
    pub soul:               Option<String>,
    pub chat_system_prompt: Option<String>,
    pub proactive_enabled:  Option<bool>,
    pub proactive_cron:     Option<String>,
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

impl Settings {
    /// Apply a partial update patch.
    pub fn apply_patch(&mut self, patch: UpdateRequest) {
        if let Some(ai) = patch.ai {
            if let Some(key) = ai.openrouter_api_key {
                self.ai.openrouter_api_key = normalize_secret(Some(key));
            }
            if let Some(model) = ai.default_model {
                self.ai.default_model = normalize_text(Some(model));
            }
            if let Some(model) = ai.job_model {
                self.ai.job_model = normalize_text(Some(model));
            }
            if let Some(model) = ai.chat_model {
                self.ai.chat_model = normalize_text(Some(model));
            }
            if let Some(favorites) = ai.favorite_models {
                self.ai.favorite_models = favorites;
            }
            if let Some(fallbacks) = ai.chat_model_fallbacks {
                self.ai.chat_model_fallbacks = fallbacks;
            }
            if let Some(fallbacks) = ai.job_model_fallbacks {
                self.ai.job_model_fallbacks = fallbacks;
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
            if let Some(soul) = agent.soul {
                self.agent.soul = normalize_text(Some(soul));
            }
            if let Some(prompt) = agent.chat_system_prompt {
                self.agent.chat_system_prompt = normalize_text(Some(prompt));
            }
            if let Some(enabled) = agent.proactive_enabled {
                self.agent.proactive_enabled = enabled;
            }
            if let Some(cron) = agent.proactive_cron {
                self.agent.proactive_cron = normalize_text(Some(cron));
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
    }

    /// Sanitize values by trimming and dropping empty strings.
    pub fn normalize(&mut self) {
        self.ai.openrouter_api_key = normalize_secret(self.ai.openrouter_api_key.take());
        self.ai.default_model = normalize_text(self.ai.default_model.take());
        self.ai.job_model = normalize_text(self.ai.job_model.take());
        self.ai.chat_model = normalize_text(self.ai.chat_model.take());
        self.ai.favorite_models.retain(|s| !s.trim().is_empty());
        self.ai.favorite_models.dedup();
        normalize_string_list(&mut self.ai.chat_model_fallbacks);
        normalize_string_list(&mut self.ai.job_model_fallbacks);
        self.telegram.bot_token = normalize_secret(self.telegram.bot_token.take());
        self.agent.soul = normalize_text(self.agent.soul.take());
        self.agent.chat_system_prompt = normalize_text(self.agent.chat_system_prompt.take());
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
    fn model_for_falls_back_to_hardcoded_default() {
        let ai = AISettings::default();
        assert_eq!(ai.model_for(ModelScenario::Job), "openai/gpt-4o");
        assert_eq!(ai.model_for(ModelScenario::Chat), "openai/gpt-4o");
    }

    #[test]
    fn model_for_uses_default_model_when_no_specific() {
        let ai = AISettings {
            default_model: Some("anthropic/claude-sonnet-4".to_owned()),
            ..Default::default()
        };
        assert_eq!(
            ai.model_for(ModelScenario::Job),
            "anthropic/claude-sonnet-4"
        );
        assert_eq!(
            ai.model_for(ModelScenario::Chat),
            "anthropic/claude-sonnet-4"
        );
    }

    #[test]
    fn model_for_uses_scenario_specific_model() {
        let ai = AISettings {
            default_model: Some("anthropic/claude-sonnet-4".to_owned()),
            job_model: Some("openai/gpt-4o".to_owned()),
            chat_model: Some("openai/gpt-4o-mini".to_owned()),
            ..Default::default()
        };
        assert_eq!(ai.model_for(ModelScenario::Job), "openai/gpt-4o");
        assert_eq!(ai.model_for(ModelScenario::Chat), "openai/gpt-4o-mini");
    }

    #[test]
    fn model_for_partial_override() {
        let ai = AISettings {
            default_model: Some("anthropic/claude-sonnet-4".to_owned()),
            job_model: Some("openai/gpt-4o".to_owned()),
            chat_model: None,
            ..Default::default()
        };
        assert_eq!(ai.model_for(ModelScenario::Job), "openai/gpt-4o");
        // Chat falls back to default_model
        assert_eq!(
            ai.model_for(ModelScenario::Chat),
            "anthropic/claude-sonnet-4"
        );
    }

    #[test]
    fn apply_patch_clears_scenario_model_with_empty_string() {
        let mut settings = Settings {
            ai: AISettings {
                job_model: Some("openai/gpt-4o".to_owned()),
                ..Default::default()
            },
            ..Default::default()
        };
        settings.apply_patch(UpdateRequest {
            ai:           Some(AiRuntimeSettingsPatch {
                job_model: Some("".to_owned()), // empty string clears
                ..Default::default()
            }),
            telegram:     None,
            agent:        None,
            job_pipeline: None,
        });
        assert_eq!(settings.ai.job_model, None);
    }

    #[test]
    fn normalize_clears_whitespace_only_models() {
        let mut settings = Settings {
            ai: AISettings {
                default_model: Some("  ".to_owned()),
                job_model: Some("  openai/gpt-4o  ".to_owned()),
                chat_model: Some("".to_owned()),
                ..Default::default()
            },
            ..Default::default()
        };
        settings.normalize();
        assert_eq!(settings.ai.default_model, None);
        assert_eq!(settings.ai.job_model, Some("openai/gpt-4o".to_owned()));
        assert_eq!(settings.ai.chat_model, None);
    }

    #[test]
    fn agent_settings_default_values() {
        let settings = Settings::default();
        assert_eq!(settings.agent.soul, None);
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
                soul:               Some("You are a cheerful assistant.".to_owned()),
                chat_system_prompt: None,
                proactive_enabled:  Some(true),
                proactive_cron:     Some("0 9 * * *".to_owned()),
                memory:             None,
                composio:           None,
                gmail:              None,
            }),
            job_pipeline: None,
        });
        assert_eq!(
            settings.agent.soul,
            Some("You are a cheerful assistant.".to_owned())
        );
        assert!(settings.agent.proactive_enabled);
        assert_eq!(settings.agent.proactive_cron, Some("0 9 * * *".to_owned()));
    }

    #[test]
    fn apply_patch_agent_partial() {
        let mut settings = Settings {
            agent: AgentSettings {
                soul:               Some("existing soul".to_owned()),
                chat_system_prompt: None,
                proactive_enabled:  true,
                proactive_cron:     Some("0 9 * * *".to_owned()),
                memory:             MemorySettings::default(),
                composio:           ComposioSettings::default(),
                gmail:              GmailSettings::default(),
            },
            ..Default::default()
        };
        // Only update proactive_enabled, leave soul and cron unchanged
        settings.apply_patch(UpdateRequest {
            ai:           None,
            telegram:     None,
            agent:        Some(AgentRuntimeSettingsPatch {
                soul:               None,
                chat_system_prompt: None,
                proactive_enabled:  Some(false),
                proactive_cron:     None,
                memory:             None,
                composio:           None,
                gmail:              None,
            }),
            job_pipeline: None,
        });
        assert_eq!(settings.agent.soul, Some("existing soul".to_owned()));
        assert!(!settings.agent.proactive_enabled);
        assert_eq!(settings.agent.proactive_cron, Some("0 9 * * *".to_owned()));
    }

    #[test]
    fn normalize_agent_settings() {
        let mut settings = Settings {
            agent: AgentSettings {
                soul:               Some("  ".to_owned()),
                chat_system_prompt: None,
                proactive_enabled:  true,
                proactive_cron:     Some("  0 9 * * *  ".to_owned()),
                memory:             MemorySettings::default(),
                composio:           ComposioSettings::default(),
                gmail:              GmailSettings::default(),
            },
            ..Default::default()
        };
        settings.normalize();
        assert_eq!(settings.agent.soul, None);
        assert_eq!(settings.agent.proactive_cron, Some("0 9 * * *".to_owned()));
    }

    #[test]
    fn agent_settings_serde_default() {
        // Deserialization of old JSON without agent field should give defaults
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
                soul:               None,
                chat_system_prompt: None,
                proactive_enabled:  None,
                proactive_cron:     None,
                memory:             Some(MemoryRuntimeSettingsPatch {
                    chroma_url:        Some("http://localhost:8000".to_owned()),
                    chroma_collection: Some("team-memory".to_owned()),
                    chroma_api_key:    Some("secret-token".to_owned()),
                }),
                composio:           None,
                gmail:              None,
            }),
            job_pipeline: None,
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

    // -- fallback_chain tests -----------------------------------------------

    #[test]
    fn fallback_chain_no_fallbacks_returns_primary_only() {
        let ai = AISettings {
            chat_model: Some("openai/gpt-4o".to_owned()),
            ..Default::default()
        };
        assert_eq!(
            ai.fallback_chain(ModelScenario::Chat),
            vec!["openai/gpt-4o"]
        );
    }

    #[test]
    fn fallback_chain_returns_correct_order() {
        let ai = AISettings {
            chat_model: Some("openai/gpt-4o".to_owned()),
            chat_model_fallbacks: vec![
                "anthropic/claude-sonnet-4".to_owned(),
                "google/gemini-2.0-flash".to_owned(),
            ],
            ..Default::default()
        };
        let chain = ai.fallback_chain(ModelScenario::Chat);
        assert_eq!(
            chain,
            vec![
                "openai/gpt-4o",
                "anthropic/claude-sonnet-4",
                "google/gemini-2.0-flash",
            ]
        );
    }

    #[test]
    fn fallback_chain_deduplicates_primary() {
        let ai = AISettings {
            job_model: Some("openai/gpt-4o".to_owned()),
            job_model_fallbacks: vec![
                "openai/gpt-4o".to_owned(), // same as primary — should be skipped
                "anthropic/claude-sonnet-4".to_owned(),
            ],
            ..Default::default()
        };
        let chain = ai.fallback_chain(ModelScenario::Job);
        assert_eq!(chain, vec!["openai/gpt-4o", "anthropic/claude-sonnet-4"]);
    }

    #[test]
    fn fallback_chain_skips_empty_entries() {
        let ai = AISettings {
            chat_model: Some("openai/gpt-4o".to_owned()),
            chat_model_fallbacks: vec![
                "".to_owned(),
                "  ".to_owned(),
                "anthropic/claude-sonnet-4".to_owned(),
            ],
            ..Default::default()
        };
        let chain = ai.fallback_chain(ModelScenario::Chat);
        assert_eq!(chain, vec!["openai/gpt-4o", "anthropic/claude-sonnet-4"]);
    }

    #[test]
    fn fallback_chain_uses_default_model_as_primary() {
        let ai = AISettings {
            default_model: Some("anthropic/claude-sonnet-4".to_owned()),
            job_model_fallbacks: vec!["openai/gpt-4o".to_owned()],
            ..Default::default()
        };
        let chain = ai.fallback_chain(ModelScenario::Job);
        assert_eq!(chain, vec!["anthropic/claude-sonnet-4", "openai/gpt-4o"]);
    }

    #[test]
    fn fallback_chain_uses_hardcoded_default_when_nothing_set() {
        let ai = AISettings {
            chat_model_fallbacks: vec!["anthropic/claude-sonnet-4".to_owned()],
            ..Default::default()
        };
        let chain = ai.fallback_chain(ModelScenario::Chat);
        assert_eq!(chain, vec!["openai/gpt-4o", "anthropic/claude-sonnet-4"]);
    }

    #[test]
    fn apply_patch_fallback_models() {
        let mut settings = Settings::default();
        settings.apply_patch(UpdateRequest {
            ai:           Some(AiRuntimeSettingsPatch {
                chat_model_fallbacks: Some(vec![
                    "anthropic/claude-sonnet-4".to_owned(),
                    "google/gemini-2.0-flash".to_owned(),
                ]),
                job_model_fallbacks: Some(vec!["openai/gpt-4o-mini".to_owned()]),
                ..Default::default()
            }),
            telegram:     None,
            agent:        None,
            job_pipeline: None,
        });
        assert_eq!(
            settings.ai.chat_model_fallbacks,
            vec!["anthropic/claude-sonnet-4", "google/gemini-2.0-flash"]
        );
        assert_eq!(settings.ai.job_model_fallbacks, vec!["openai/gpt-4o-mini"]);
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
                soul:               None,
                chat_system_prompt: None,
                proactive_enabled:  None,
                proactive_cron:     None,
                memory:             None,
                composio:           None,
                gmail:              Some(GmailRuntimeSettingsPatch {
                    address:           Some("user@gmail.com".to_owned()),
                    app_password:      Some("abcd-efgh-ijkl-mnop".to_owned()),
                    auto_send_enabled: Some(true),
                }),
            }),
            job_pipeline: None,
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
    }

    #[test]
    fn normalize_trims_and_deduplicates_fallbacks() {
        let mut settings = Settings {
            ai: AISettings {
                chat_model_fallbacks: vec![
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
            settings.ai.chat_model_fallbacks,
            vec!["openai/gpt-4o", "anthropic/claude-sonnet-4"]
        );
    }
}
