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

//! Job-fit analysis agent.

use rig::{client::CompletionClient, completion::Prompt, providers::openrouter};

use crate::error::AiError;

const SYSTEM_PROMPT: &str = "\
You are a career advisor. Analyze the job posting and the candidate's resume, then produce a \
                             structured fit assessment including:
- A fit score from 0 to 100
- Key strengths that match the role
- Gaps or areas of concern
- A brief summary of the overall fit";

/// Evaluates how well a candidate's resume matches a job posting.
pub struct JobFitAgent {
    client: openrouter::Client,
    model:  String,
}

impl JobFitAgent {
    pub(crate) fn new(client: openrouter::Client, model: String) -> Self { Self { client, model } }

    /// Analyze the fit between a job description and a resume.
    pub async fn analyze(&self, job_description: &str, resume: &str) -> Result<String, AiError> {
        let user_input = format!("## Job Description\n{job_description}\n\n## Resume\n{resume}");

        let agent = self
            .client
            .agent(&self.model)
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
