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

//! Job posting analyzer agent.

use crate::{agents::prompt::compose_system_prompt, client::LlmClient, error::AiError};

const SYSTEM_PROMPT_FILE: &str = "ai/jd_analyzer.system.md";
const DEFAULT_SYSTEM_PROMPT: &str = include_str!("../../../../prompts/ai/jd_analyzer.system.md");

/// Analyzes a job posting in markdown format and extracts structured
/// information using AI.
pub struct JdAnalyzerAgent {
    client:      LlmClient,
    model:       String,
    soul_prompt: Option<String>,
}

impl JdAnalyzerAgent {
    pub(crate) fn new(
        client: LlmClient,
        model: String,
        soul_prompt: Option<String>,
    ) -> Self {
        Self {
            client,
            model,
            soul_prompt,
        }
    }

    /// Analyze a job posting markdown and return structured JSON.
    pub async fn analyze(&self, markdown: &str) -> Result<String, AiError> {
        let base_prompt =
            rara_paths::load_prompt_markdown(SYSTEM_PROMPT_FILE, DEFAULT_SYSTEM_PROMPT);
        let system_prompt = compose_system_prompt(&base_prompt, self.soul_prompt.as_deref());

        self.client
            .run_agent(&self.model, &system_prompt, markdown)
            .await
    }
}
