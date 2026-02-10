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

//! Resume optimization agent.

use rig::{client::CompletionClient, completion::Prompt, providers::openrouter};

use crate::error::AiError;

const SYSTEM_PROMPT: &str = "\
You are a professional resume writer. Rewrite the resume to better match the target job \
                             description while keeping all facts accurate. Focus on:
- Highlighting relevant experience and skills
- Using keywords from the job description
- Improving clarity and impact of bullet points
- Maintaining professional formatting";

/// Optimizes a resume for a specific job posting.
pub struct ResumeOptimizerAgent<'a> {
    client: &'a openrouter::Client,
    model:  &'a str,
}

impl<'a> ResumeOptimizerAgent<'a> {
    pub(crate) fn new(client: &'a openrouter::Client, model: &'a str) -> Self { Self { client, model } }

    /// Optimize a resume to better match a job description.
    pub async fn optimize(&self, resume: &str, job_description: &str) -> Result<String, AiError> {
        let user_input =
            format!("## Current Resume\n{resume}\n\n## Target Job Description\n{job_description}");

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
