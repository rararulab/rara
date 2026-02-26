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

//! Job fit analysis agent.
//!
//! Supports optional tool-calling mode: when a [`ToolRegistry`] is provided,
//! the agent can invoke tools (e.g. query user profile, past applications)
//! during its analysis loop.

use std::sync::Arc;

use agent_core::{provider::LlmProvider, tool_registry::ToolRegistry};

use crate::builtin::tasks::{
    completion::{DEFAULT_TASK_TOOL_ITERATIONS, run_completion, run_with_tools},
    error::TaskAgentError,
};

/// Evaluates how well a candidate's resume matches a job posting.
pub struct JobFitAgent {
    provider:       Arc<dyn LlmProvider>,
    model:          String,
    prompt_repo:    Arc<dyn agent_core::prompt::PromptRepo>,
    tools:          Option<Arc<ToolRegistry>>,
    max_iterations: usize,
}

impl JobFitAgent {
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

    /// Analyze the fit between a job description and a resume.
    pub async fn analyze(
        &self,
        job_description: &str,
        resume: &str,
    ) -> Result<String, TaskAgentError> {
        let user_input = format!("## Job Description\n{job_description}\n\n## Resume\n{resume}");
        let system_prompt = self.build_system_prompt().await;

        match &self.tools {
            Some(tools) if !tools.is_empty() => {
                run_with_tools(
                    &*self.provider,
                    &self.model,
                    &system_prompt,
                    &user_input,
                    tools,
                    self.max_iterations,
                )
                .await
            }
            _ => run_completion(&*self.provider, &self.model, &system_prompt, &user_input).await,
        }
    }

    async fn build_system_prompt(&self) -> String {
        let base = self
            .prompt_repo
            .get("ai/job_fit.system.md")
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
