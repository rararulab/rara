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

//! Interview preparation agent.

use rig::{client::CompletionClient, completion::Prompt, providers::openrouter};

use crate::error::AiError;

const SYSTEM_PROMPT: &str = "\
You are an interview coach. Generate likely interview questions and suggested answers based on the \
                             job description and resume. Include:
- Technical questions relevant to the role
- Behavioral questions (STAR format answers)
- Questions the candidate should ask the interviewer
- Tips for preparation";

/// Generates interview preparation materials.
pub struct InterviewPrepAgent {
    client: openrouter::Client,
    model:  String,
}

impl InterviewPrepAgent {
    pub(crate) fn new(client: openrouter::Client, model: String) -> Self { Self { client, model } }

    /// Generate interview preparation materials.
    pub async fn prepare(&self, job_description: &str, resume: &str) -> Result<String, AiError> {
        let user_input = format!("## Job Description\n{job_description}\n\n## My Resume\n{resume}");

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
