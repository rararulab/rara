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

//! Resume analysis agent.
//!
//! Evaluates a resume across multiple dimensions and provides actionable
//! optimization suggestions.

use rig::{client::CompletionClient, completion::Prompt, providers::openrouter};

use crate::error::AiError;

const SYSTEM_PROMPT: &str = "\
You are an expert resume consultant with deep knowledge of ATS (Applicant Tracking Systems), \
hiring practices, and professional resume writing. Analyze resumes thoroughly and provide \
actionable, specific feedback. Be constructive but honest about weaknesses. Format your \
response in clear markdown with scores and bullet points.";

/// Analyzes a resume and provides a structured report with scores and
/// improvement suggestions.
pub struct ResumeAnalyzerAgent {
    client: openrouter::Client,
    model:  String,
}

impl ResumeAnalyzerAgent {
    pub(crate) fn new(client: openrouter::Client, model: String) -> Self { Self { client, model } }

    /// Analyze a resume (with optional job context) and return a structured
    /// report.
    pub async fn analyze(&self, prompt: &str) -> Result<String, AiError> {
        let agent = self
            .client
            .agent(&self.model)
            .preamble(SYSTEM_PROMPT)
            .build();

        agent
            .prompt(prompt)
            .await
            .map_err(|e| AiError::RequestFailed {
                message: e.to_string(),
            })
    }
}
