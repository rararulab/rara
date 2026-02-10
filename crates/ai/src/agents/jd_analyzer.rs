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

use rig::{client::CompletionClient, completion::Prompt, providers::openai};

use crate::error::AiError;

const SYSTEM_PROMPT: &str = "\
You are a job posting analyzer. Given a job posting in markdown format, analyze it and return ONLY \
a valid JSON object with these fields:
- title (string, required - the job title)
- company (string, required - the company name)
- location (string or null - work location)
- employment_type (string or null - e.g. \"full-time\", \"contract\")
- experience_level (string or null - e.g. \"senior\", \"mid\", \"junior\")
- salary_range (string or null - salary info if mentioned)
- required_skills (array of strings - required skills/technologies)
- preferred_skills (array of strings - nice-to-have skills)
- responsibilities (array of strings - key responsibilities, max 5)
- requirements (array of strings - key requirements, max 5)
- benefits (array of strings - benefits mentioned, max 5)
- summary (string - a 2-3 sentence summary of the role)
- match_score (integer 0-100 - estimated attractiveness for a senior software engineer)

Return ONLY the JSON object, no other text.";

/// Analyzes a job posting in markdown format and extracts structured
/// information using AI.
pub struct JdAnalyzerAgent<'a> {
    client: &'a openai::Client,
    model:  &'a str,
}

impl<'a> JdAnalyzerAgent<'a> {
    pub(crate) fn new(client: &'a openai::Client, model: &'a str) -> Self {
        Self { client, model }
    }

    /// Analyze a job posting markdown and return structured JSON.
    pub async fn analyze(&self, markdown: &str) -> Result<String, AiError> {
        let agent = self
            .client
            .agent(self.model)
            .preamble(SYSTEM_PROMPT)
            .build();

        agent
            .prompt(markdown)
            .await
            .map_err(|e| AiError::RequestFailed {
                message: e.to_string(),
            })
    }
}
