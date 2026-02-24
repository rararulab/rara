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

//! Follow-up email drafting agent.

use std::sync::Arc;

use agent_core::provider::LlmProvider;

use crate::builtin::tasks::{completion::run_completion, error::TaskAgentError};

/// Drafts follow-up emails after interviews or applications.
pub struct FollowUpDraftAgent {
    provider:    Arc<dyn LlmProvider>,
    model:       String,
    prompt_repo: Arc<dyn rara_prompt::PromptRepo>,
}

impl FollowUpDraftAgent {
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

    /// Draft a follow-up email based on the given context.
    pub async fn draft(&self, context: &str) -> Result<String, TaskAgentError> {
        let base = self.prompt_repo.get("ai/follow_up.system.md").await
            .map(|e| e.content)
            .unwrap_or_default();
        let soul = rara_prompt::resolve_soul(self.prompt_repo.as_ref(), None).await;
        let system_prompt = rara_prompt::compose_with_soul(&base, soul.as_deref(), "Task Instructions");

        run_completion(&*self.provider, &self.model, &system_prompt, context).await
    }
}
