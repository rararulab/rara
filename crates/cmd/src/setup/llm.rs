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

use snafu::{ResultExt, Whatever};

use super::prompt::{self, SetupMode};

/// LLM configuration result.
pub struct LlmResult {
    /// Provider name (e.g. "openrouter", "ollama", "custom").
    pub provider_name: String,
    /// Base URL for the OpenAI-compatible API.
    pub base_url:      String,
    /// API key (may be placeholder for local providers).
    pub api_key:       String,
    /// Default model identifier.
    pub default_model: String,
}

/// Guide the user through LLM provider configuration.
pub async fn setup_llm(
    existing: Option<&rara_app::flatten::LlmConfig>,
    mode: SetupMode,
) -> Result<Option<LlmResult>, Whatever> {
    prompt::print_step("LLM Provider");

    if mode == SetupMode::FillMissing && existing.is_some() {
        prompt::print_ok("already configured, skipping");
        return Ok(None);
    }

    loop {
        let provider_idx = prompt::ask_choice(
            "Provider type:",
            &[
                "OpenRouter (recommended)",
                "Ollama (local)",
                "Custom OpenAI-compatible",
            ],
        );

        let (provider_name, default_url, needs_key) = match provider_idx {
            0 => ("openrouter", "https://openrouter.ai/api/v1", true),
            1 => ("ollama", "http://localhost:11434/v1", false),
            _ => ("custom", "http://localhost:8080/v1", true),
        };

        let base_url = prompt::ask("Base URL", Some(default_url));

        let api_key = if needs_key {
            prompt::ask_password("API Key")
        } else {
            prompt::ask("API Key", Some("ollama"))
        };

        let default_model_hint = match provider_idx {
            0 => "anthropic/claude-sonnet-4",
            1 => "qwen3:32b",
            _ => "",
        };
        let default_model = prompt::ask("Default model", Some(default_model_hint));

        // Validate by listing models from the provider's /models endpoint.
        match validate_llm(&base_url, &api_key).await {
            Ok(()) => prompt::print_ok(&format!("API verified (model: {default_model})")),
            Err(e) => {
                prompt::print_err(&format!("API check failed: {e}"));
                if !prompt::confirm("Continue anyway?", false) {
                    continue;
                }
            }
        }

        return Ok(Some(LlmResult {
            provider_name: provider_name.to_owned(),
            base_url,
            api_key,
            default_model,
        }));
    }
}

/// Validate LLM provider by hitting the /models endpoint.
async fn validate_llm(base_url: &str, api_key: &str) -> Result<(), Whatever> {
    let client = reqwest::Client::new();
    let url = format!("{}/models", base_url.trim_end_matches('/'));

    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .whatever_context("request failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        snafu::whatever!("server returned {status}");
    }

    Ok(())
}
