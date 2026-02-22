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

//! Provider-agnostic LLM client wrapper.
//!
//! [`LlmClient`] abstracts over different rig-core providers (OpenRouter,
//! Ollama) so that agent code does not need to know which backend is in use.

use rig::{
    client::CompletionClient,
    completion::Prompt,
    providers::{ollama, openrouter},
};

use crate::error::AiError;

/// Provider-agnostic LLM client wrapper.
#[derive(Clone)]
pub(crate) enum LlmClient {
    OpenRouter(openrouter::Client),
    Ollama(ollama::Client),
}

impl LlmClient {
    /// Build a rig agent with the given model + system prompt, then run it.
    pub async fn run_agent(
        &self,
        model: &str,
        preamble: &str,
        user_input: &str,
    ) -> Result<String, AiError> {
        match self {
            Self::OpenRouter(c) => {
                let agent = c.agent(model).preamble(preamble).build();
                agent
                    .prompt(user_input)
                    .await
                    .map_err(|e| AiError::RequestFailed {
                        message: e.to_string(),
                    })
            }
            Self::Ollama(c) => {
                let agent = c.agent(model).preamble(preamble).build();
                agent
                    .prompt(user_input)
                    .await
                    .map_err(|e| AiError::RequestFailed {
                        message: e.to_string(),
                    })
            }
        }
    }
}
