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

//! Shared model types used across the agent runner.

use base::shared_string::SharedString;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmProviderFamily {
    OpenRouter,
    Ollama,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelCapabilities {
    pub provider:             LlmProviderFamily,
    pub supports_tools:       bool,
    pub tools_disabled_reason: Option<&'static str>,
}

impl ModelCapabilities {
    #[must_use]
    pub fn detect(provider_hint: Option<&str>, model_name: &str) -> Self {
        let provider = detect_provider_family(provider_hint, model_name);
        let canonical = canonical_model_name(model_name);

        // Ollama serves many raw models whose chat templates/tool-calling support
        // varies. Keep the deny-list small and explicit so unsupported models
        // degrade gracefully without breaking tool-capable ones.
        if matches!(provider, LlmProviderFamily::Ollama)
            && canonical.starts_with("deepseek-r1")
        {
            return Self {
                provider,
                supports_tools: false,
                tools_disabled_reason: Some(
                    "ollama deepseek-r1 variants do not support function/tool calling",
                ),
            };
        }

        Self {
            provider,
            supports_tools: true,
            tools_disabled_reason: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id:        SharedString,
    pub name:      SharedString,
    pub arguments: serde_json::Value,
}

fn detect_provider_family(provider_hint: Option<&str>, model_name: &str) -> LlmProviderFamily {
    let provider_hint = provider_hint.map(str::trim).map(str::to_ascii_lowercase);
    match provider_hint.as_deref() {
        Some("ollama") => return LlmProviderFamily::Ollama,
        Some("openrouter") => return LlmProviderFamily::OpenRouter,
        _ => {}
    }

    let trimmed = model_name.trim();
    // Common Ollama local model syntax: `name:tag` with no provider prefix.
    if trimmed.contains(':') && !trimmed.contains('/') {
        return LlmProviderFamily::Ollama;
    }

    LlmProviderFamily::Unknown
}

fn canonical_model_name(model_name: &str) -> String {
    let trimmed = model_name.trim().to_ascii_lowercase();
    trimmed
        .rsplit('/')
        .next()
        .unwrap_or(trimmed.as_str())
        .to_owned()
}

// Re-exports from provider module for backward compatibility.
pub use crate::provider::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_ollama_deepseek_r1_as_no_tools() {
        let caps = ModelCapabilities::detect(Some("ollama"), "deepseek-r1:14b");
        assert_eq!(caps.provider, LlmProviderFamily::Ollama);
        assert!(!caps.supports_tools);
        assert!(caps.tools_disabled_reason.is_some());
    }

    #[test]
    fn detects_ollama_registry_prefix_as_no_tools() {
        let caps = ModelCapabilities::detect(
            Some("ollama"),
            "registry.ollama.ai/library/deepseek-r1:14b",
        );
        assert!(!caps.supports_tools);
    }

    #[test]
    fn infers_ollama_from_local_tagged_name() {
        let caps = ModelCapabilities::detect(None, "deepseek-r1:14b");
        assert_eq!(caps.provider, LlmProviderFamily::Ollama);
        assert!(!caps.supports_tools);
    }

    #[test]
    fn leaves_other_models_tool_capable_by_default() {
        let caps = ModelCapabilities::detect(Some("openrouter"), "google/gemini-2.0-flash");
        assert_eq!(caps.provider, LlmProviderFamily::OpenRouter);
        assert!(caps.supports_tools);
        assert!(caps.tools_disabled_reason.is_none());
    }
}
