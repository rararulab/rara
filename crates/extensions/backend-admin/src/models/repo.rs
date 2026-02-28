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

//! [`ModelRepo`] implementation backed by [`SettingsSvc`].

use std::collections::HashMap;

use rara_domain_shared::settings::model::{AiRuntimeSettingsPatch, UpdateRequest};
use rara_kernel::model_repo::{HARDCODED_DEFAULT_MODEL, ModelEntry, ModelRepo, ModelRepoError};

use crate::settings::SettingsSvc;

/// [`ModelRepo`] implementation that reads/writes model configuration
/// through the runtime settings service.
pub struct SettingsModelRepo {
    settings_svc: SettingsSvc,
}

impl SettingsModelRepo {
    pub fn new(settings_svc: SettingsSvc) -> Self { Self { settings_svc } }
}

#[async_trait::async_trait]
impl ModelRepo for SettingsModelRepo {
    async fn get(&self, key: &str) -> String {
        let settings = self.settings_svc.current();
        settings
            .ai
            .models
            .get(key)
            .or_else(|| settings.ai.models.get("default"))
            .cloned()
            .unwrap_or_else(|| HARDCODED_DEFAULT_MODEL.to_owned())
    }

    async fn set(&self, key: &str, model: &str) -> Result<(), ModelRepoError> {
        let patch = UpdateRequest {
            ai:           Some(AiRuntimeSettingsPatch {
                models: Some(HashMap::from([(key.to_owned(), Some(model.to_owned()))])),
                ..Default::default()
            }),
            telegram:     None,
            agent:        None,
            job_pipeline: None,
            workers:      None,
        };
        self.settings_svc
            .update(patch)
            .await
            .map_err(|e| ModelRepoError::Persistence {
                message: e.to_string(),
            })?;
        Ok(())
    }

    async fn remove(&self, key: &str) -> Result<(), ModelRepoError> {
        let patch = UpdateRequest {
            ai:           Some(AiRuntimeSettingsPatch {
                models: Some(HashMap::from([(key.to_owned(), None)])),
                ..Default::default()
            }),
            telegram:     None,
            agent:        None,
            job_pipeline: None,
            workers:      None,
        };
        self.settings_svc
            .update(patch)
            .await
            .map_err(|e| ModelRepoError::Persistence {
                message: e.to_string(),
            })?;
        Ok(())
    }

    async fn list(&self) -> Vec<ModelEntry> {
        let settings = self.settings_svc.current();
        settings
            .ai
            .models
            .iter()
            .map(|(k, v)| ModelEntry {
                key:   k.clone(),
                model: v.clone(),
            })
            .collect()
    }

    async fn fallback_models(&self) -> Vec<String> {
        self.settings_svc.current().ai.fallback_models.clone()
    }

    async fn set_fallback_models(&self, models: Vec<String>) -> Result<(), ModelRepoError> {
        let patch = UpdateRequest {
            ai:           Some(AiRuntimeSettingsPatch {
                fallback_models: Some(models),
                ..Default::default()
            }),
            telegram:     None,
            agent:        None,
            job_pipeline: None,
            workers:      None,
        };
        self.settings_svc
            .update(patch)
            .await
            .map_err(|e| ModelRepoError::Persistence {
                message: e.to_string(),
            })?;
        Ok(())
    }
}
