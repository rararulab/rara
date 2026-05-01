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
//! # Resolution contract (`resolve_agent`, introduced in #1635)
//!
//! Agents declare their LLM binding in YAML under
//! `agents.<name>.{driver, model}`. The registry's
//! [`DriverRegistry::resolve_agent`] returns a [`ResolvedAgent`] with
//! the driver instance, the exact model, and a manifest snapshot, so
//! callers never see a driver resolved one way and a model resolved
//! another. Priority:
//!
//! ```text
//! Driver: agents.<name>.driver > agent_overrides > manifest.provider_hint > default_driver
//! Model:  agents.<name>.model  > agent_overrides > manifest.model         > provider_models[driver].default_model
//! ```
//!
//! # Legacy `resolve` (kept as a thin shim)
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

use super::{
    catalog::OpenRouterCatalog,
    driver::{LlmDriverRef, LlmEmbedderRef, LlmModelListerRef},
};
use crate::{agent::AgentManifest, error};

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
    /// Per-provider `LlmModelLister` instances. Populated alongside
    /// `drivers` for providers that expose a `/models` endpoint
    /// (currently every `OpenAiDriver`-backed provider). Looked up by
    /// `runtime_lister` at call time so a settings-driven swap of
    /// `default_driver` immediately routes the next chat-model fetch to
    /// the new provider.
    listers:         HashMap<String, LlmModelListerRef>,
    /// Per-provider `LlmEmbedder` instances. Same lifecycle as
    /// `listers` — registered when the driver is registered and read
    /// dynamically by `runtime_embedder` so swaps take effect without a
    /// restart.
    embedders:       HashMap<String, LlmEmbedderRef>,
    default_driver:  String,
    provider_models: HashMap<String, ProviderModelConfig>,
    agent_overrides: HashMap<String, AgentDriverConfig>,
    /// Per-agent `{driver, model}` pair loaded from `agents.<name>.*` YAML.
    ///
    /// This is the new unified source of truth introduced by the agent
    /// registry refactor (issue #1635). Existing `agent_overrides` remain
    /// until consumers migrate in follow-up issues.
    agent_configs:   HashMap<String, AgentLlmConfig>,
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

/// Unified per-agent LLM config read from `agents.<name>.*` YAML.
///
/// Unlike [`AgentDriverConfig`] (which is a legacy override layered on
/// top of a manifest), this represents the full `{driver, model}` pair
/// that [`DriverRegistry::resolve_agent`] will return for an agent.
#[derive(Debug, Clone, Default)]
pub struct AgentLlmConfig {
    /// Driver name (e.g., `"openrouter"`, `"ollama"`).
    pub driver: Option<String>,
    /// Model identifier (e.g., `"qwen3:32b"`).
    pub model:  Option<String>,
}

/// Fully-resolved LLM binding for an agent.
///
/// Produced by [`DriverRegistry::resolve_agent`]. Holds the driver
/// instance, the exact model identifier, and a clone of the agent's
/// manifest so callers have a single consistent snapshot — eliminating
/// the historical split between a driver resolved one way and a model
/// string resolved another (see issue #1635).
#[derive(Clone, bon::Builder)]
pub struct ResolvedAgent {
    /// Shared driver instance capable of serving the agent's requests.
    pub driver:   LlmDriverRef,
    /// Concrete model identifier to pass to the driver.
    pub model:    String,
    /// Snapshot of the manifest used for resolution.
    pub manifest: AgentManifest,
}

/// Named driver map with default selection and per-agent overrides.
pub struct DriverRegistry {
    state:   RwLock<DriverRegistryState>,
    catalog: Arc<OpenRouterCatalog>,
}

impl DriverRegistry {
    /// Create an empty registry with the given default driver name and
    /// a shared [`OpenRouterCatalog`] for model capability lookups.
    pub fn new(default_driver: impl Into<String>, catalog: Arc<OpenRouterCatalog>) -> Self {
        Self {
            state: RwLock::new(DriverRegistryState {
                drivers:         HashMap::new(),
                listers:         HashMap::new(),
                embedders:       HashMap::new(),
                default_driver:  default_driver.into(),
                provider_models: HashMap::new(),
                agent_overrides: HashMap::new(),
                agent_configs:   HashMap::new(),
            }),
            catalog,
        }
    }

    /// Access the shared model capability catalog.
    pub fn catalog(&self) -> &OpenRouterCatalog { &self.catalog }

    /// Register or replace a named driver instance.
    pub fn register_driver(&self, name: impl Into<String>, driver: LlmDriverRef) {
        let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
        state.drivers.insert(name.into(), driver);
    }

    /// Register or replace a named [`LlmModelLister`](super::LlmModelLister).
    ///
    /// Boot wires this for every provider whose driver implements
    /// `LlmModelLister` (today: every `OpenAiDriver`-backed provider).
    /// `RuntimeModelLister` looks up the entry at call time using
    /// [`Self::default_driver`], so a settings-driven swap of the
    /// default provider routes the next `list_models` call to the new
    /// provider's catalog with no restart and no boot-time `Arc` capture
    /// to invalidate.
    pub fn register_lister(&self, name: impl Into<String>, lister: LlmModelListerRef) {
        let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
        state.listers.insert(name.into(), lister);
    }

    /// Register or replace a named [`LlmEmbedder`](super::LlmEmbedder).
    ///
    /// Same lifecycle as [`Self::register_lister`] — `RuntimeEmbedder`
    /// resolves the active embedder per call so the knowledge layer
    /// follows the active provider without re-instantiation.
    pub fn register_embedder(&self, name: impl Into<String>, embedder: LlmEmbedderRef) {
        let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
        state.embedders.insert(name.into(), embedder);
    }

    /// Look up the lister for a registered provider, if any.
    pub fn get_lister(&self, name: &str) -> Option<LlmModelListerRef> {
        self.state
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .listers
            .get(name)
            .cloned()
    }

    /// Look up the embedder for a registered provider, if any.
    pub fn get_embedder(&self, name: &str) -> Option<LlmEmbedderRef> {
        self.state
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .embedders
            .get(name)
            .cloned()
    }

    /// Update the active default driver name.
    ///
    /// Called from the `PATCH /api/v1/settings` handler when
    /// `llm.default_provider` changes so the next call into
    /// `RuntimeModelLister` / `RuntimeEmbedder` resolves through the
    /// new provider. Setting the default driver to a name that has not
    /// been registered is allowed (mirrors the `register_driver`
    /// out-of-order policy) — resolution will surface
    /// `ProviderNotConfigured` until a matching driver is registered.
    pub fn set_default_driver(&self, name: impl Into<String>) {
        let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
        state.default_driver = name.into();
    }

    /// Set model configuration for a provider.
    pub fn set_provider_model(
        &self,
        name: impl Into<String>,
        default_model: impl Into<String>,
        fallback_models: Vec<String>,
    ) {
        let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
        state.provider_models.insert(
            name.into(),
            ProviderModelConfig {
                default_model: default_model.into(),
                fallback_models,
            },
        );
    }

    /// Set a per-agent override.
    pub fn set_agent_override(&self, agent_name: impl Into<String>, config: AgentDriverConfig) {
        let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
        state.agent_overrides.insert(agent_name.into(), config);
    }

    /// Set the unified `{driver, model}` config for an agent, as loaded
    /// from `agents.<name>.*` YAML.
    ///
    /// Used by [`Self::resolve_agent`]. Independent of the legacy
    /// [`Self::set_agent_override`] map — callers may populate either or
    /// both during the migration period.
    pub fn set_agent_config(&self, agent_name: impl Into<String>, config: AgentLlmConfig) {
        let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
        state.agent_configs.insert(agent_name.into(), config);
    }

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

    /// Resolve a [`ResolvedAgent`] for the given manifest — the unified
    /// `{driver, model, manifest}` entry point introduced by #1635.
    ///
    /// Resolution priority:
    /// - **Driver**: `agents.<name>.driver` > `agent_overrides[name].driver`
    ///   > `manifest.provider_hint` > `default_driver`
    /// - **Model**:  `agents.<name>.model`  > `agent_overrides[name].model`
    ///   > `manifest.model` > `provider_models[driver].default_model`
    ///
    /// The agent name is taken from `manifest.name`. The returned
    /// `ResolvedAgent` carries a clone of the manifest so callers have a
    /// single consistent snapshot — closing the historical split where
    /// the driver was resolved via the registry but the model came from
    /// a flat settings key (see the `knowledge_extractor` prod failure
    /// that motivated this refactor).
    pub fn resolve_agent(&self, manifest: &AgentManifest) -> error::Result<ResolvedAgent> {
        let state = self.state.read().unwrap_or_else(|e| e.into_inner());
        let agent_name = manifest.name.as_str();
        let agent_cfg = state.agent_configs.get(agent_name);
        let legacy_override = state.agent_overrides.get(agent_name);

        let driver_name = agent_cfg
            .and_then(|c| c.driver.as_deref())
            .or_else(|| legacy_override.and_then(|c| c.driver.as_deref()))
            .or(manifest.provider_hint.as_deref())
            .unwrap_or(&state.default_driver);

        let provider_default = state
            .provider_models
            .get(driver_name)
            .map(|c| c.default_model.as_str());

        let model_name = agent_cfg
            .and_then(|c| c.model.as_deref())
            .or_else(|| legacy_override.and_then(|c| c.model.as_deref()))
            .or(manifest.model.as_deref())
            .or(provider_default)
            .unwrap_or("unknown");

        let driver = state
            .drivers
            .get(driver_name)
            .context(error::ProviderNotConfiguredSnafu)?;

        Ok(ResolvedAgent {
            driver:   Arc::clone(driver),
            model:    model_name.to_string(),
            manifest: manifest.clone(),
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agent::{AgentRole, Priority},
        llm::ScriptedLlmDriver,
    };

    fn manifest(name: &str) -> AgentManifest {
        AgentManifest {
            name:                   name.to_string(),
            role:                   AgentRole::Chat,
            description:            "test".to_string(),
            model:                  None,
            system_prompt:          "sp".to_string(),
            soul_prompt:            None,
            provider_hint:          None,
            max_iterations:         None,
            tools:                  Vec::new(),
            excluded_tools:         Vec::new(),
            max_children:           None,
            max_context_tokens:     None,
            priority:               Priority::default(),
            metadata:               serde_json::Value::Null,
            sandbox:                None,
            default_execution_mode: None,
            tool_call_limit:        None,
            worker_timeout_secs:    None,
            max_continuations:      None,
            max_output_chars:       None,
        }
    }

    fn registry_with_providers() -> DriverRegistry {
        let catalog = Arc::new(OpenRouterCatalog::new());
        let reg = DriverRegistry::new("openrouter", catalog);
        reg.register_driver("openrouter", Arc::new(ScriptedLlmDriver::new(Vec::new())));
        reg.register_driver("ollama", Arc::new(ScriptedLlmDriver::new(Vec::new())));
        reg.set_provider_model("openrouter", "gpt-4o", Vec::<String>::new());
        reg.set_provider_model("ollama", "qwen3:32b", Vec::<String>::new());
        reg
    }

    #[test]
    fn resolve_agent_returns_agent_specific_pair() {
        let reg = registry_with_providers();
        reg.set_agent_config(
            "knowledge_extractor",
            AgentLlmConfig {
                driver: Some("ollama".into()),
                model:  Some("qwen3:14b".into()),
            },
        );

        let m = manifest("knowledge_extractor");
        let resolved = reg.resolve_agent(&m).expect("resolve_agent");
        assert_eq!(resolved.model, "qwen3:14b");
        assert_eq!(resolved.manifest.name, "knowledge_extractor");
    }

    #[test]
    fn resolve_agent_falls_back_to_manifest_then_provider_default() {
        let reg = registry_with_providers();

        // Manifest-only model, no per-agent YAML config.
        let mut m = manifest("rara");
        m.model = Some("gpt-4o-mini".into());
        let resolved = reg.resolve_agent(&m).expect("resolve_agent");
        assert_eq!(resolved.model, "gpt-4o-mini");

        // Pure fallback to provider default.
        let m_empty = manifest("blank");
        let resolved = reg.resolve_agent(&m_empty).expect("resolve_agent");
        assert_eq!(resolved.model, "gpt-4o");
    }

    #[test]
    fn resolve_agent_errors_when_driver_unknown() {
        let catalog = Arc::new(OpenRouterCatalog::new());
        let reg = DriverRegistry::new("missing", catalog);
        // No driver registered.
        let m = manifest("ghost");
        let result = reg.resolve_agent(&m);
        assert!(result.is_err(), "expected error for unknown driver");
        match result {
            Err(crate::error::KernelError::ProviderNotConfigured { .. }) => {}
            other => panic!("unexpected result variant: {:?}", other.err()),
        }
    }

    /// #1670: when no YAML entry and no settings override exist for an
    /// agent, `resolve_agent` must fall through to the default provider's
    /// default model — the same inheritance chain the main agent uses.
    #[test]
    fn resolve_agent_inherits_default_when_no_entry() {
        let reg = registry_with_providers();

        // `blank` has no agent_config, no agent_override, no manifest model
        // and no manifest provider_hint. The resolver must pick the default
        // driver ("openrouter") and its default model ("gpt-4o").
        let m = manifest("blank");
        let resolved = reg.resolve_agent(&m).expect("resolve_agent");
        assert_eq!(resolved.model, "gpt-4o");
    }

    /// #1670: runtime settings override wins over YAML — i.e. mutating the
    /// `agent_config` entry between `resolve_agent` calls takes effect on
    /// the next call without re-constructing the registry.
    #[test]
    fn resolve_agent_runtime_override_takes_precedence() {
        let reg = registry_with_providers();

        // Baseline: fallback to provider default.
        let m = manifest("knowledge_extractor");
        assert_eq!(
            reg.resolve_agent(&m).expect("baseline").model,
            "gpt-4o",
            "without any override, inherit provider default"
        );

        // Simulate a YAML entry being synced into the registry.
        reg.set_agent_config(
            "knowledge_extractor",
            AgentLlmConfig {
                driver: Some("ollama".into()),
                model:  Some("qwen3:32b".into()),
            },
        );
        assert_eq!(
            reg.resolve_agent(&m).expect("yaml").model,
            "qwen3:32b",
            "YAML entry wins over provider default"
        );

        // Simulate a runtime mutation (settings.db write hot-reloaded into
        // the registry) replacing the YAML entry with a new pair.
        reg.set_agent_config(
            "knowledge_extractor",
            AgentLlmConfig {
                driver: Some("openrouter".into()),
                model:  Some("gpt-4o-mini".into()),
            },
        );
        let resolved = reg.resolve_agent(&m).expect("runtime override");
        assert_eq!(
            resolved.model, "gpt-4o-mini",
            "runtime mutation picked up on next resolve_agent call"
        );
    }

    #[test]
    fn legacy_resolve_shim_still_works() {
        let reg = registry_with_providers();
        reg.set_agent_override(
            "rara",
            AgentDriverConfig {
                driver: Some("ollama".into()),
                model:  Some("qwen3:32b".into()),
            },
        );

        let (driver, model) = reg.resolve("rara", None, None).expect("resolve");
        assert_eq!(model, "qwen3:32b");
        // Driver reference is non-null — smoke check that the Arc resolved.
        drop(driver);

        // Unknown agent still falls back to the default driver + its default
        // model, proving the legacy path is untouched by the new API.
        let (_, model) = reg.resolve("unknown", None, None).expect("resolve");
        assert_eq!(model, "gpt-4o");
    }
}
