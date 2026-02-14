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
    pub ai:         AISettings,
    pub telegram:   TelegramSettings,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

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
    pub openrouter_api_key: Option<String>,
    pub default_model:      Option<String>,
    pub job_model:          Option<String>,
    pub chat_model:         Option<String>,
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
}

/// Telegram-specific runtime settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TelegramSettings {
    pub bot_token: Option<String>,
    pub chat_id:   Option<i64>,
}

/// Partial update payload for runtime settings writes.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateRequest {
    pub ai:       Option<AiRuntimeSettingsPatch>,
    pub telegram: Option<TelegramRuntimeSettingsPatch>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiRuntimeSettingsPatch {
    pub openrouter_api_key: Option<String>,
    pub default_model:      Option<String>,
    /// `Some(model)` to set, `None` to leave unchanged.
    /// Use `Some("")` or send an empty string to clear (revert to default).
    pub job_model:          Option<String>,
    /// `Some(model)` to set, `None` to leave unchanged.
    /// Use `Some("")` or send an empty string to clear (revert to default).
    pub chat_model:         Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TelegramRuntimeSettingsPatch {
    pub bot_token: Option<String>,
    pub chat_id:   Option<i64>,
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
        }

        if let Some(telegram) = patch.telegram {
            if let Some(token) = telegram.bot_token {
                self.telegram.bot_token = normalize_secret(Some(token));
            }
            if let Some(chat_id) = telegram.chat_id {
                self.telegram.chat_id = Some(chat_id);
            }
        }
    }

    /// Sanitize values by trimming and dropping empty strings.
    pub fn normalize(&mut self) {
        self.ai.openrouter_api_key = normalize_secret(self.ai.openrouter_api_key.take());
        self.ai.default_model = normalize_text(self.ai.default_model.take());
        self.ai.job_model = normalize_text(self.ai.job_model.take());
        self.ai.chat_model = normalize_text(self.ai.chat_model.take());
        self.telegram.bot_token = normalize_secret(self.telegram.bot_token.take());
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
        assert_eq!(ai.model_for(ModelScenario::Job), "anthropic/claude-sonnet-4");
        assert_eq!(ai.model_for(ModelScenario::Chat), "anthropic/claude-sonnet-4");
    }

    #[test]
    fn model_for_uses_scenario_specific_model() {
        let ai = AISettings {
            default_model: Some("anthropic/claude-sonnet-4".to_owned()),
            job_model:     Some("openai/gpt-4o".to_owned()),
            chat_model:    Some("openai/gpt-4o-mini".to_owned()),
            ..Default::default()
        };
        assert_eq!(ai.model_for(ModelScenario::Job), "openai/gpt-4o");
        assert_eq!(ai.model_for(ModelScenario::Chat), "openai/gpt-4o-mini");
    }

    #[test]
    fn model_for_partial_override() {
        let ai = AISettings {
            default_model: Some("anthropic/claude-sonnet-4".to_owned()),
            job_model:     Some("openai/gpt-4o".to_owned()),
            chat_model:    None,
            ..Default::default()
        };
        assert_eq!(ai.model_for(ModelScenario::Job), "openai/gpt-4o");
        // Chat falls back to default_model
        assert_eq!(ai.model_for(ModelScenario::Chat), "anthropic/claude-sonnet-4");
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
            ai:       Some(AiRuntimeSettingsPatch {
                job_model: Some("".to_owned()), // empty string clears
                ..Default::default()
            }),
            telegram: None,
        });
        assert_eq!(settings.ai.job_model, None);
    }

    #[test]
    fn normalize_clears_whitespace_only_models() {
        let mut settings = Settings {
            ai: AISettings {
                default_model: Some("  ".to_owned()),
                job_model:     Some("  openai/gpt-4o  ".to_owned()),
                chat_model:    Some("".to_owned()),
                ..Default::default()
            },
            ..Default::default()
        };
        settings.normalize();
        assert_eq!(settings.ai.default_model, None);
        assert_eq!(settings.ai.job_model, Some("openai/gpt-4o".to_owned()));
        assert_eq!(settings.ai.chat_model, None);
    }
}
