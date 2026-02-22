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

//! Cover letter generation agent.

use crate::{agents::prompt::compose_system_prompt, client::LlmClient, error::AiError};

const SYSTEM_PROMPT_FILE: &str = "ai/cover_letter.system.md";
const DEFAULT_SYSTEM_PROMPT: &str = include_str!("../../../../prompts/ai/cover_letter.system.md");

/// Generates cover letters tailored to job postings.
pub struct CoverLetterAgent {
    client:      LlmClient,
    model:       String,
    soul_prompt: Option<String>,
}

impl CoverLetterAgent {
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

    /// Generate a cover letter for the given job description and resume.
    pub async fn generate(&self, job_description: &str, resume: &str) -> Result<String, AiError> {
        let user_input = format!("## Job Description\n{job_description}\n\n## My Resume\n{resume}");
        let base_prompt =
            rara_paths::load_prompt_markdown(SYSTEM_PROMPT_FILE, DEFAULT_SYSTEM_PROMPT);
        let system_prompt = compose_system_prompt(&base_prompt, self.soul_prompt.as_deref());

        self.client
            .run_agent(&self.model, &system_prompt, &user_input)
            .await
    }
}
