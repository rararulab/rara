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
//! Model:  agent_overrides > manifest.model          > default_model
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

#[derive(Clone)]
struct DriverRegistryState {
    drivers:         HashMap<String, LlmDriverRef>,
    default_driver:  String,
    default_model:   String,
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
    ///   `default_model`
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

        let model_name = override_cfg
            .and_then(|c| c.model.as_deref())
            .or(manifest_model)
            .unwrap_or(&state.default_model);

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

    /// Get the default model name.
    pub fn default_model(&self) -> String {
        self.state
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .default_model
            .clone()
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
    default_model:   String,
    agent_overrides: HashMap<String, AgentDriverConfig>,
}

impl DriverRegistryBuilder {
    /// Create a new builder with the given default driver and model names.
    pub fn new(default_driver: impl Into<String>, default_model: impl Into<String>) -> Self {
        Self {
            drivers:         HashMap::new(),
            default_driver:  default_driver.into(),
            default_model:   default_model.into(),
            agent_overrides: HashMap::new(),
        }
    }

    /// Register a named driver instance.
    pub fn driver(mut self, name: impl Into<String>, driver: LlmDriverRef) -> Self {
        self.drivers.insert(name.into(), driver);
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
                default_model:   self.default_model,
                agent_overrides: self.agent_overrides,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use tokio::sync::mpsc;

    use super::DriverRegistryBuilder;
    use crate::{
        error::Result,
        llm::{
            driver::LlmDriver,
            stream::StreamDelta,
            types::{CompletionRequest, CompletionResponse},
        },
    };

    struct TestDriver;

    #[async_trait]
    impl LlmDriver for TestDriver {
        async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse> {
            unreachable!("not used in registry tests")
        }

        async fn stream(
            &self,
            _request: CompletionRequest,
            _tx: mpsc::Sender<StreamDelta>,
        ) -> Result<CompletionResponse> {
            unreachable!("not used in registry tests")
        }
    }

    #[test]
    fn update_replaces_default_driver_and_registered_drivers() {
        let registry = Arc::new(
            DriverRegistryBuilder::new("openrouter", "model-a")
                .driver("openrouter", Arc::new(TestDriver))
                .build(),
        );

        let updated = DriverRegistryBuilder::new("ollama", "model-b")
            .driver("ollama", Arc::new(TestDriver))
            .build();

        registry.update(&updated);

        let (_, model) = registry
            .resolve("agent", None, None)
            .expect("driver should resolve");
        assert_eq!(registry.default_driver(), "ollama");
        assert_eq!(registry.default_model(), "model-b");
        assert_eq!(model, "model-b");
        assert_eq!(registry.driver_names(), vec!["ollama".to_string()]);
    }
}
