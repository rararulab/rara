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

use rig::{client::CompletionClient, completion::Prompt, providers::openrouter};

use crate::error::AiError;

const SYSTEM_PROMPT: &str = "\
You are a professional cover-letter writer. Craft a compelling cover letter that highlights the \
                             candidate's relevant experience for the role. The letter should:
- Be tailored to the specific position
- Highlight 2-3 key qualifications
- Show enthusiasm for the company and role
- Be concise (under 400 words)";

/// Generates cover letters tailored to job postings.
pub struct CoverLetterAgent<'a> {
    client: &'a openrouter::Client,
    model:  &'a str,
}

impl<'a> CoverLetterAgent<'a> {
    pub(crate) fn new(client: &'a openrouter::Client, model: &'a str) -> Self {
        Self { client, model }
    }

    /// Generate a cover letter for the given job description and resume.
    pub async fn generate(&self, job_description: &str, resume: &str) -> Result<String, AiError> {
        let user_input = format!("## Job Description\n{job_description}\n\n## My Resume\n{resume}");

        let agent = self
            .client
            .agent(self.model)
            .preamble(SYSTEM_PROMPT)
            .build();

        agent
            .prompt(&user_input)
            .await
            .map_err(|e| AiError::RequestFailed {
                message: e.to_string(),
            })
    }
}
