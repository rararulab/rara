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

use std::{collections::HashMap, sync::Arc};

use snafu::OptionExt;

use super::driver::LlmDriverRef;
use crate::error;

/// Shared reference to the [`DriverRegistry`].
pub type DriverRegistryRef = Arc<DriverRegistry>;

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
    drivers:         HashMap<String, LlmDriverRef>,
    default_driver:  String,
    default_model:   String,
    agent_overrides: HashMap<String, AgentDriverConfig>,
}

impl std::fmt::Debug for DriverRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let driver_names: Vec<&str> = self.drivers.keys().map(|s| s.as_str()).collect();
        f.debug_struct("DriverRegistry")
            .field("drivers", &driver_names)
            .field("default_driver", &self.default_driver)
            .field("default_model", &self.default_model)
            .field("agent_overrides", &self.agent_overrides)
            .finish()
    }
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
        let override_cfg = self.agent_overrides.get(agent_name);

        let driver_name = override_cfg
            .and_then(|c| c.driver.as_deref())
            .or(manifest_provider_hint)
            .unwrap_or(&self.default_driver);

        let model_name = override_cfg
            .and_then(|c| c.model.as_deref())
            .or(manifest_model)
            .unwrap_or(&self.default_model);

        let driver = self
            .drivers
            .get(driver_name)
            .context(error::ProviderNotConfiguredSnafu)?;

        Ok((Arc::clone(driver), model_name.to_string()))
    }

    /// Get the default driver name.
    pub fn default_driver(&self) -> &str { &self.default_driver }

    /// Get the default model name.
    pub fn default_model(&self) -> &str { &self.default_model }

    /// List all registered driver names.
    pub fn driver_names(&self) -> Vec<&str> { self.drivers.keys().map(|s| s.as_str()).collect() }
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
            drivers:         self.drivers,
            default_driver:  self.default_driver,
            default_model:   self.default_model,
            agent_overrides: self.agent_overrides,
        }
    }
}
