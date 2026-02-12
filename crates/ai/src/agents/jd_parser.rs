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

//! Job description parser agent.

use rig::{client::CompletionClient, completion::Prompt, providers::openrouter};

use crate::error::AiError;

const SYSTEM_PROMPT: &str =
    "\
You are a job description parser. Given a raw job description text, extract structured information \
     and return ONLY a valid JSON object with these fields:\n- title (string, required)\n- \
     company (string, required)\n- location (string or null)\n- description (string or null - a \
     clean summary)\n- url (string or null)\n- salary_min (integer or null)\n- salary_max \
     (integer or null)\n- salary_currency (string or null, e.g. \"USD\")\n- tags (array of \
     strings - relevant skills/keywords)\n\nReturn ONLY the JSON object, no other text.";

/// Parses raw job description text into structured JSON using AI.
pub struct JdParserAgent {
    client: openrouter::Client,
    model:  String,
}

impl JdParserAgent {
    pub(crate) fn new(client: openrouter::Client, model: String) -> Self { Self { client, model } }

    /// Parse a raw job description into a structured JSON string.
    pub async fn parse(&self, jd_text: &str) -> Result<String, AiError> {
        let agent = self
            .client
            .agent(&self.model)
            .preamble(SYSTEM_PROMPT)
            .build();

        agent
            .prompt(jd_text)
            .await
            .map_err(|e| AiError::RequestFailed {
                message: e.to_string(),
            })
    }
}
