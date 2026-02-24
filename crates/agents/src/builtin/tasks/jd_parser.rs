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
//!
//! Supports optional tool-calling mode: when a [`ToolRegistry`] is provided,
//! the agent can invoke tools (e.g. validate parsed data) during parsing.

use std::sync::Arc;

use agent_core::provider::LlmProvider;
use agent_core::tool_registry::ToolRegistry;

use crate::builtin::tasks::completion::{
    run_completion, run_with_tools, DEFAULT_TASK_TOOL_ITERATIONS,
};
use crate::builtin::tasks::error::TaskAgentError;

/// Parses raw job description text into structured JSON using AI.
pub struct JdParserAgent {
    provider:        Arc<dyn LlmProvider>,
    model:           String,
    prompt_repo:     Arc<dyn agent_core::prompt::PromptRepo>,
    tools:           Option<Arc<ToolRegistry>>,
    max_iterations:  usize,
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
            tools: None,
            max_iterations: DEFAULT_TASK_TOOL_ITERATIONS,
        }
    }

    /// Attach a tool registry for tool-calling mode.
    #[must_use]
    pub fn with_tools(mut self, tools: Arc<ToolRegistry>) -> Self {
        self.tools = Some(tools);
        self
    }

    /// Override the maximum number of tool-calling iterations.
    #[must_use]
    pub fn with_max_iterations(mut self, max: usize) -> Self {
        self.max_iterations = max;
        self
    }

    /// Parse a raw job description into a structured JSON string.
    pub async fn parse(&self, jd_text: &str) -> Result<String, TaskAgentError> {
        let system_prompt = self.build_system_prompt().await;

        match &self.tools {
            Some(tools) if !tools.is_empty() => {
                run_with_tools(
                    &*self.provider,
                    &self.model,
                    &system_prompt,
                    jd_text,
                    tools,
                    self.max_iterations,
                )
                .await
            }
            _ => run_completion(&*self.provider, &self.model, &system_prompt, jd_text).await,
        }
    }

    async fn build_system_prompt(&self) -> String {
        let base = self
            .prompt_repo
            .get("ai/jd_parser.system.md")
            .await
            .map(|e| e.content)
            .unwrap_or_default();
        let soul = self
            .prompt_repo
            .get("agent/soul.md")
            .await
            .map(|e| e.content)
            .unwrap_or_default();
        if soul.trim().is_empty() {
            base
        } else {
            format!("{soul}\n\n# Task Instructions\n{base}")
        }
    }
}
