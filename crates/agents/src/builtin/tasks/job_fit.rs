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

use std::sync::Arc;

use agent_core::provider::LlmProvider;

use crate::builtin::tasks::{completion::run_completion, error::TaskAgentError};

/// Evaluates how well a candidate's resume matches a job posting.
pub struct JobFitAgent {
    provider:    Arc<dyn LlmProvider>,
    model:       String,
    prompt_repo: Arc<dyn rara_prompt::PromptRepo>,
}

impl JobFitAgent {
    pub(crate) fn new(
        provider: Arc<dyn LlmProvider>,
        model: String,
        prompt_repo: Arc<dyn rara_prompt::PromptRepo>,
    ) -> Self {
        Self {
            provider,
            model,
            prompt_repo,
        }
    }

    /// Analyze the fit between a job description and a resume.
    pub async fn analyze(
        &self,
        job_description: &str,
        resume: &str,
    ) -> Result<String, TaskAgentError> {
        let user_input = format!("## Job Description\n{job_description}\n\n## Resume\n{resume}");
        let base = self.prompt_repo.get("ai/job_fit.system.md").await
            .map(|e| e.content)
            .unwrap_or_default();
        let soul = rara_prompt::resolve_soul(self.prompt_repo.as_ref(), None).await;
        let system_prompt = rara_prompt::compose_with_soul(&base, soul.as_deref(), "Task Instructions");

        run_completion(&*self.provider, &self.model, &system_prompt, &user_input).await
    }
}
