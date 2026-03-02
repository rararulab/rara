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

//! Multi-provider LLM registry with per-agent override support.
//!
//! [`ProviderRegistry`] replaces the single-provider `LlmProviderLoader`
//! trait with a named provider map, default selection, and per-agent
//! overrides. Resolution priority:
//!
//! ```text
//! Provider: agent_overrides > manifest.provider_hint > default_provider
//! Model:    agent_overrides > manifest.model          > default_model
//! ```

use std::{collections::HashMap, sync::Arc};

use snafu::OptionExt;

use super::LlmProvider;
use crate::error;

/// Per-agent LLM configuration override.
///
/// Both fields are optional — `None` means "fall through to the next
/// priority level" (manifest, then global default).
#[derive(Debug, Clone, Default)]
pub struct AgentLlmConfig {
    /// Override provider name (e.g., `"ollama"`, `"openrouter"`).
    pub provider: Option<String>,
    /// Override model identifier (e.g., `"qwen3:32b"`).
    pub model:    Option<String>,
}

/// Named provider map with default selection and per-agent overrides.
///
/// Thread-safe and cheaply cloneable via `Arc` wrapping.
pub struct ProviderRegistry {
    /// Named provider instances (e.g., `"openrouter"` -> OpenAiProvider).
    providers:        HashMap<String, Arc<dyn LlmProvider>>,
    /// Default provider name (must exist in `providers`).
    default_provider: String,
    /// Default model identifier.
    default_model:    String,
    /// Per-agent overrides keyed by agent manifest name.
    agent_overrides:  HashMap<String, AgentLlmConfig>,
}

impl std::fmt::Debug for ProviderRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let provider_names: Vec<&str> = self.providers.keys().map(|s| s.as_str()).collect();
        f.debug_struct("ProviderRegistry")
            .field("providers", &provider_names)
            .field("default_provider", &self.default_provider)
            .field("default_model", &self.default_model)
            .field("agent_overrides", &self.agent_overrides)
            .finish()
    }
}

impl ProviderRegistry {
    /// Resolve a provider + model for a given agent.
    ///
    /// Resolution priority:
    /// - **Provider**: `agent_overrides[name].provider` >
    ///   `manifest_provider_hint` > `default_provider`
    /// - **Model**: `agent_overrides[name].model` > `manifest_model` >
    ///   `default_model`
    ///
    /// Returns `(Arc<dyn LlmProvider>, model_name)` or an error if the
    /// resolved provider name is not registered.
    pub fn resolve(
        &self,
        agent_name: &str,
        manifest_provider_hint: Option<&str>,
        manifest_model: Option<&str>,
    ) -> error::Result<(Arc<dyn LlmProvider>, String)> {
        let override_cfg = self.agent_overrides.get(agent_name);

        // Resolve provider name
        let provider_name = override_cfg
            .and_then(|c| c.provider.as_deref())
            .or(manifest_provider_hint)
            .unwrap_or(&self.default_provider);

        // Resolve model name
        let model_name = override_cfg
            .and_then(|c| c.model.as_deref())
            .or(manifest_model)
            .unwrap_or(&self.default_model);

        let provider = self
            .providers
            .get(provider_name)
            .context(error::ProviderNotConfiguredSnafu)?;

        Ok((Arc::clone(provider), model_name.to_string()))
    }

    /// Get the default provider name.
    pub fn default_provider(&self) -> &str { &self.default_provider }

    /// Get the default model name.
    pub fn default_model(&self) -> &str { &self.default_model }

    /// List all registered provider names.
    pub fn provider_names(&self) -> Vec<&str> {
        self.providers.keys().map(|s| s.as_str()).collect()
    }
}

/// Builder for constructing a [`ProviderRegistry`].
///
/// # Example
///
/// ```rust,ignore
/// let registry = ProviderRegistryBuilder::new("openrouter", "openai/gpt-4o-mini")
///     .provider("openrouter", Arc::new(openrouter_provider))
///     .provider("ollama", Arc::new(ollama_provider))
///     .agent_override("rara", AgentLlmConfig {
///         provider: Some("ollama".to_string()),
///         model: Some("qwen3:32b".to_string()),
///     })
///     .build();
/// ```
pub struct ProviderRegistryBuilder {
    providers:        HashMap<String, Arc<dyn LlmProvider>>,
    default_provider: String,
    default_model:    String,
    agent_overrides:  HashMap<String, AgentLlmConfig>,
}

impl ProviderRegistryBuilder {
    /// Create a new builder with the given default provider and model names.
    pub fn new(default_provider: impl Into<String>, default_model: impl Into<String>) -> Self {
        Self {
            providers:        HashMap::new(),
            default_provider: default_provider.into(),
            default_model:    default_model.into(),
            agent_overrides:  HashMap::new(),
        }
    }

    /// Register a named provider instance.
    pub fn provider(mut self, name: impl Into<String>, provider: Arc<dyn LlmProvider>) -> Self {
        self.providers.insert(name.into(), provider);
        self
    }

    /// Register a per-agent LLM override.
    pub fn agent_override(mut self, agent_name: impl Into<String>, config: AgentLlmConfig) -> Self {
        self.agent_overrides.insert(agent_name.into(), config);
        self
    }

    /// Build the [`ProviderRegistry`].
    pub fn build(self) -> ProviderRegistry {
        ProviderRegistry {
            providers:        self.providers,
            default_provider: self.default_provider,
            default_model:    self.default_model,
            agent_overrides:  self.agent_overrides,
        }
    }
}

#[cfg(test)]
mod tests {
    use async_openai::types::chat::{
        ChatCompletionResponseStream, CreateChatCompletionRequest, CreateChatCompletionResponse,
    };
    use async_trait::async_trait;

    use super::*;

    /// Minimal stub provider for testing.
    struct StubProvider {
        name: String,
    }

    impl StubProvider {
        fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
            }
        }
    }

    impl std::fmt::Debug for StubProvider {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "StubProvider({})", self.name)
        }
    }

    #[async_trait]
    impl LlmProvider for StubProvider {
        async fn chat_completion(
            &self,
            _request: CreateChatCompletionRequest,
        ) -> error::Result<CreateChatCompletionResponse> {
            Err(crate::error::KernelError::Other {
                message: "stub".into(),
            })
        }

        async fn chat_completion_stream(
            &self,
            _request: CreateChatCompletionRequest,
        ) -> error::Result<ChatCompletionResponseStream> {
            Err(crate::error::KernelError::Other {
                message: "stub".into(),
            })
        }
    }

    fn make_registry() -> ProviderRegistry {
        ProviderRegistryBuilder::new("openrouter", "openai/gpt-4o-mini")
            .provider("openrouter", Arc::new(StubProvider::new("openrouter")))
            .provider("ollama", Arc::new(StubProvider::new("ollama")))
            .agent_override(
                "rara",
                AgentLlmConfig {
                    provider: Some("ollama".to_string()),
                    model:    Some("qwen3:32b".to_string()),
                },
            )
            .agent_override(
                "scout",
                AgentLlmConfig {
                    provider: None,
                    model:    Some("deepseek/deepseek-chat".to_string()),
                },
            )
            .build()
    }

    #[test]
    fn resolve_default_when_no_overrides() {
        let reg = make_registry();
        let (_, model) = reg.resolve("unknown-agent", None, None).unwrap();
        assert_eq!(model, "openai/gpt-4o-mini");
    }

    #[test]
    fn resolve_manifest_model_overrides_default() {
        let reg = make_registry();
        let (_, model) = reg.resolve("unknown-agent", None, Some("gpt-4")).unwrap();
        assert_eq!(model, "gpt-4");
    }

    #[test]
    fn resolve_manifest_provider_hint_overrides_default() {
        let reg = make_registry();
        let (_, model) = reg
            .resolve("unknown-agent", Some("ollama"), Some("llama3"))
            .unwrap();
        assert_eq!(model, "llama3");
    }

    #[test]
    fn resolve_agent_override_takes_priority() {
        let reg = make_registry();
        // "rara" has provider=ollama, model=qwen3:32b override
        let (_, model) = reg
            .resolve("rara", Some("openrouter"), Some("gpt-4"))
            .unwrap();
        assert_eq!(model, "qwen3:32b");
    }

    #[test]
    fn resolve_partial_agent_override_falls_through() {
        let reg = make_registry();
        // "scout" has model override but no provider override
        let (_, model) = reg.resolve("scout", None, None).unwrap();
        assert_eq!(model, "deepseek/deepseek-chat");
    }

    #[test]
    fn resolve_unknown_provider_errors() {
        let reg = ProviderRegistryBuilder::new("nonexistent", "model").build();
        let result = reg.resolve("agent", None, None);
        assert!(result.is_err());
    }

    #[test]
    fn provider_names_lists_all() {
        let reg = make_registry();
        let mut names = reg.provider_names();
        names.sort();
        assert_eq!(names, vec!["ollama", "openrouter"]);
    }

    #[test]
    fn default_accessors() {
        let reg = make_registry();
        assert_eq!(reg.default_provider(), "openrouter");
        assert_eq!(reg.default_model(), "openai/gpt-4o-mini");
    }
}
