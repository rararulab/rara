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

//! Job description parser agent.

use std::sync::Arc;

use agent_core::provider::LlmProvider;

use crate::builtin::tasks::{completion::run_completion, error::TaskAgentError};

/// Parses raw job description text into structured JSON using AI.
pub struct JdParserAgent {
    provider:    Arc<dyn LlmProvider>,
    model:       String,
    prompt_repo: Arc<dyn agent_core::prompt::PromptRepo>,
}

impl JdParserAgent {
    pub(crate) fn new(
        provider: Arc<dyn LlmProvider>,
        model: String,
        prompt_repo: Arc<dyn agent_core::prompt::PromptRepo>,
    ) -> Self {
        Self {
            provider,
            model,
            prompt_repo,
        }
    }

    /// Parse a raw job description into a structured JSON string.
    pub async fn parse(&self, jd_text: &str) -> Result<String, TaskAgentError> {
        let base = self.prompt_repo.get("ai/jd_parser.system.md").await
            .map(|e| e.content)
            .unwrap_or_default();
        let soul = self.prompt_repo.get("agent/soul.md").await
            .map(|e| e.content)
            .unwrap_or_default();
        let system_prompt = if soul.trim().is_empty() {
            base
        } else {
            format!("{soul}\n\n# Task Instructions\n{base}")
        };

        run_completion(&*self.provider, &self.model, &system_prompt, jd_text).await
    }
}
