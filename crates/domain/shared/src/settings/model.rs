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

/// AI-specific runtime settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AISettings {
    pub openrouter_api_key: Option<String>,
    pub model:              Option<String>,
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
    pub model:              Option<String>,
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
            if let Some(model) = ai.model {
                self.ai.model = normalize_text(Some(model));
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
        self.ai.model = normalize_text(self.ai.model.take());
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
