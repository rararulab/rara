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

//! Resume analysis agent.
//!
//! Evaluates a resume across multiple dimensions and provides actionable
//! optimization suggestions.

use rig::{client::CompletionClient, completion::Prompt, providers::openrouter};

use crate::{agents::prompt::compose_system_prompt, error::AiError};

const SYSTEM_PROMPT_FILE: &str = "ai/resume_analyzer.system.md";
const DEFAULT_SYSTEM_PROMPT: &str =
    include_str!("../../../../prompts/ai/resume_analyzer.system.md");

/// Analyzes a resume and provides a structured report with scores and
/// improvement suggestions.
pub struct ResumeAnalyzerAgent {
    client:      openrouter::Client,
    model:       String,
    soul_prompt: Option<String>,
}

impl ResumeAnalyzerAgent {
    pub(crate) fn new(
        client: openrouter::Client,
        model: String,
        soul_prompt: Option<String>,
    ) -> Self {
        Self {
            client,
            model,
            soul_prompt,
        }
    }

    /// Analyze a resume (with optional job context) and return a structured
    /// report.
    pub async fn analyze(&self, prompt: &str) -> Result<String, AiError> {
        let base_prompt =
            rara_paths::load_prompt_markdown(SYSTEM_PROMPT_FILE, DEFAULT_SYSTEM_PROMPT);
        let system_prompt = compose_system_prompt(&base_prompt, self.soul_prompt.as_deref());
        let agent = self
            .client
            .agent(&self.model)
            .preamble(&system_prompt)
            .build();

        agent
            .prompt(prompt)
            .await
            .map_err(|e| AiError::RequestFailed {
                message: e.to_string(),
            })
    }
}
