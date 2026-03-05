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

//! Multi-driver LLM registry with per-agent override support.
//!
//! [`DriverRegistry`] manages named [`LlmDriver`](super::LlmDriver) instances
//! with per-agent override support for driver and model selection.
//!
//! Resolution priority:
//!
//! ```text
//! Driver: agent_overrides > manifest.provider_hint > default_driver
//! Model:  agent_overrides > manifest.model          > provider_models[driver].default_model
//! ```

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use snafu::OptionExt;

use super::driver::LlmDriverRef;
use crate::error;

/// Shared reference to the [`DriverRegistry`].
pub type DriverRegistryRef = Arc<DriverRegistry>;

/// Per-provider model configuration (default + fallbacks).
#[derive(Debug, Clone)]
pub struct ProviderModelConfig {
    /// The default model for this provider.
    pub default_model:   String,
    /// Fallback models to try when the default is unavailable.
    pub fallback_models: Vec<String>,
}

#[derive(Clone)]
struct DriverRegistryState {
    drivers:         HashMap<String, LlmDriverRef>,
    default_driver:  String,
    provider_models: HashMap<String, ProviderModelConfig>,
    agent_overrides: HashMap<String, AgentDriverConfig>,
}

/// Per-agent LLM driver configuration override.
///
/// Both fields are optional — `None` means "fall through to the next
/// priority level" (manifest, then global default).
#[derive(Debug, Clone, Default)]
pub struct AgentDriverConfig {
    /// Override driver name (e.g., `"openrouter"`, `"ollama"`).
    pub driver: Option<String>,
    /// Override model identifier (e.g., `"qwen3:32b"`).
    pub model:  Option<String>,
}

/// Named driver map with default selection and per-agent overrides.
pub struct DriverRegistry {
    state: RwLock<DriverRegistryState>,
}

impl DriverRegistry {
    /// Resolve a driver + model for a given agent.
    ///
    /// Resolution priority:
    /// - **Driver**: `agent_overrides[name].driver` > `manifest_provider_hint`
    ///   > `default_driver`
    /// - **Model**: `agent_overrides[name].model` > `manifest_model` >
    ///   `provider_models[driver].default_model`
    pub fn resolve(
        &self,
        agent_name: &str,
        manifest_provider_hint: Option<&str>,
        manifest_model: Option<&str>,
    ) -> error::Result<(LlmDriverRef, String)> {
        let state = self.state.read().unwrap_or_else(|e| e.into_inner());
        let override_cfg = state.agent_overrides.get(agent_name);

        let driver_name = override_cfg
            .and_then(|c| c.driver.as_deref())
            .or(manifest_provider_hint)
            .unwrap_or(&state.default_driver);

        let provider_default = state
            .provider_models
            .get(driver_name)
            .map(|c| c.default_model.as_str());

        let model_name = override_cfg
            .and_then(|c| c.model.as_deref())
            .or(manifest_model)
            .or(provider_default)
            .unwrap_or("unknown");

        let driver = state
            .drivers
            .get(driver_name)
            .context(error::ProviderNotConfiguredSnafu)?;

        Ok((Arc::clone(driver), model_name.to_string()))
    }

    /// Get the default driver name.
    pub fn default_driver(&self) -> String {
        self.state
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .default_driver
            .clone()
    }

    /// Get the default model for the given provider, if configured.
    pub fn default_model_for(&self, provider: &str) -> Option<String> {
        self.state
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .provider_models
            .get(provider)
            .map(|c| c.default_model.clone())
    }

    /// Get the default model for the default provider.
    ///
    /// Convenience shorthand equivalent to
    /// `default_model_for(&self.default_driver())`.
    pub fn default_model(&self) -> Option<String> {
        let state = self.state.read().unwrap_or_else(|e| e.into_inner());
        state
            .provider_models
            .get(&state.default_driver)
            .map(|c| c.default_model.clone())
    }

    /// Get the fallback models for the given provider, if configured.
    pub fn fallback_models_for(&self, provider: &str) -> Vec<String> {
        self.state
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .provider_models
            .get(provider)
            .map(|c| c.fallback_models.clone())
            .unwrap_or_default()
    }

    /// List all registered driver names.
    pub fn driver_names(&self) -> Vec<String> {
        self.state
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .drivers
            .keys()
            .cloned()
            .collect()
    }

    /// Atomically replace the current registry contents with a newly built
    /// snapshot.
    pub fn update(&self, next: &DriverRegistry) {
        let next_state = next.state.read().unwrap_or_else(|e| e.into_inner()).clone();
        let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
        *state = next_state;
    }
}

/// Builder for constructing a [`DriverRegistry`].
pub struct DriverRegistryBuilder {
    drivers:         HashMap<String, LlmDriverRef>,
    default_driver:  String,
    provider_models: HashMap<String, ProviderModelConfig>,
    agent_overrides: HashMap<String, AgentDriverConfig>,
}

impl DriverRegistryBuilder {
    /// Create a new builder with the given default driver name.
    pub fn new(default_driver: impl Into<String>) -> Self {
        Self {
            drivers:         HashMap::new(),
            default_driver:  default_driver.into(),
            provider_models: HashMap::new(),
            agent_overrides: HashMap::new(),
        }
    }

    /// Register a named driver instance.
    pub fn driver(mut self, name: impl Into<String>, driver: LlmDriverRef) -> Self {
        self.drivers.insert(name.into(), driver);
        self
    }

    /// Register model configuration for a specific provider.
    pub fn provider_model(
        mut self,
        name: impl Into<String>,
        default_model: impl Into<String>,
        fallback_models: Vec<String>,
    ) -> Self {
        self.provider_models.insert(
            name.into(),
            ProviderModelConfig {
                default_model: default_model.into(),
                fallback_models,
            },
        );
        self
    }

    /// Register a per-agent driver override.
    pub fn agent_override(
        mut self,
        agent_name: impl Into<String>,
        config: AgentDriverConfig,
    ) -> Self {
        self.agent_overrides.insert(agent_name.into(), config);
        self
    }

    /// Build the [`DriverRegistry`].
    pub fn build(self) -> DriverRegistry {
        DriverRegistry {
            state: RwLock::new(DriverRegistryState {
                drivers:         self.drivers,
                default_driver:  self.default_driver,
                provider_models: self.provider_models,
                agent_overrides: self.agent_overrides,
            }),
        }
    }
}
