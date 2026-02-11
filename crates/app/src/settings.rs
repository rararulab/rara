// Copyright 2026 Crrow
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

//! DB-backed runtime settings service with in-process cache.

use std::sync::{Arc, RwLock};

use job_domain_shared::runtime_settings::{
    RUNTIME_SETTINGS_KV_KEY, RuntimeSettings, RuntimeSettingsPatch,
};
use snafu::{ResultExt, Whatever, whatever};
use yunara_store::KVStore;

#[derive(Clone)]
pub struct RuntimeSettingsService {
    kv:    KVStore,
    cache: Arc<RwLock<RuntimeSettings>>,
}

impl RuntimeSettingsService {
    pub async fn load(kv: KVStore, fallback: RuntimeSettings) -> Result<Self, Whatever> {
        let mut stored = kv
            .get::<RuntimeSettings>(RUNTIME_SETTINGS_KV_KEY)
            .await
            .whatever_context("failed to load runtime settings from kv")?
            .unwrap_or_default();
        stored.normalize();
        let merged = stored.with_fallback(&fallback);
        Ok(Self {
            kv,
            cache: Arc::new(RwLock::new(merged)),
        })
    }

    pub fn current(&self) -> RuntimeSettings {
        self.cache
            .read()
            .map_or_else(|_| RuntimeSettings::default(), |g| g.clone())
    }

    pub async fn update(&self, patch: RuntimeSettingsPatch) -> Result<RuntimeSettings, Whatever> {
        let mut next = self.current();
        next.apply_patch(patch);
        next.normalize();

        self.kv
            .set(RUNTIME_SETTINGS_KV_KEY, &next)
            .await
            .whatever_context("failed to persist runtime settings to kv")?;

        let mut guard = match self.cache.write() {
            Ok(guard) => guard,
            Err(_) => {
                whatever!("failed to lock runtime settings cache")
            }
        };
        *guard = next.clone();
        Ok(next)
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RuntimeSettingsView {
    pub ai:       AiSettingsView,
    pub telegram: TelegramSettingsView,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AiSettingsView {
    pub configured: bool,
    pub model:      Option<String>,
    pub key_hint:   Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TelegramSettingsView {
    pub configured: bool,
    pub chat_id:    Option<i64>,
    pub token_hint: Option<String>,
}

#[must_use]
pub fn to_view(settings: &RuntimeSettings) -> RuntimeSettingsView {
    RuntimeSettingsView {
        ai:       AiSettingsView {
            configured: settings.ai.openrouter_api_key.is_some(),
            model:      settings.ai.model.clone(),
            key_hint:   secret_hint(settings.ai.openrouter_api_key.as_deref()),
        },
        telegram: TelegramSettingsView {
            configured: settings.telegram.bot_token.is_some()
                && settings.telegram.chat_id.is_some(),
            chat_id:    settings.telegram.chat_id,
            token_hint: secret_hint(settings.telegram.bot_token.as_deref()),
        },
    }
}

fn secret_hint(secret: Option<&str>) -> Option<String> {
    let secret = secret?;
    let chars: Vec<char> = secret.chars().collect();
    if chars.is_empty() {
        return None;
    }
    if chars.len() <= 4 {
        return Some("*".repeat(chars.len()));
    }
    let suffix: String = chars[chars.len() - 4..].iter().collect();
    Some(format!("***{suffix}"))
}
