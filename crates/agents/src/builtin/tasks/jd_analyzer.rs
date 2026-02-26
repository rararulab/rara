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
//!
//! Supports optional tool-calling mode: when a [`ToolRegistry`] is provided,
//! the agent can invoke tools during its analysis of job postings.

use std::sync::Arc;

use agent_core::{provider::LlmProvider, tool_registry::ToolRegistry};

use crate::builtin::tasks::{
    completion::{DEFAULT_TASK_TOOL_ITERATIONS, run_completion, run_with_tools},
    error::TaskAgentError,
};

/// Analyzes a job posting in markdown format and extracts structured
/// information using AI.
pub struct JdAnalyzerAgent {
    provider:       Arc<dyn LlmProvider>,
    model:          String,
    prompt_repo:    Arc<dyn agent_core::prompt::PromptRepo>,
    tools:          Option<Arc<ToolRegistry>>,
    max_iterations: usize,
}

impl JdAnalyzerAgent {
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

    /// Analyze a job posting markdown and return structured JSON.
    pub async fn analyze(&self, markdown: &str) -> Result<String, TaskAgentError> {
        let system_prompt = self.build_system_prompt().await;

        match &self.tools {
            Some(tools) if !tools.is_empty() => {
                run_with_tools(
                    &*self.provider,
                    &self.model,
                    &system_prompt,
                    markdown,
                    tools,
                    self.max_iterations,
                )
                .await
            }
            _ => run_completion(&*self.provider, &self.model, &system_prompt, markdown).await,
        }
    }

    async fn build_system_prompt(&self) -> String {
        let base = self
            .prompt_repo
            .get("ai/jd_analyzer.system.md")
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
