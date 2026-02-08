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

//! AI task kinds and their default configurations.
//!
//! Each [`AiTaskKind`] represents a high-level job that the platform can
//! delegate to an LLM. The kind determines which prompt template and
//! output schema are used by default.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use strum_macros::{Display, EnumIter, EnumString};

/// The category of AI task to perform.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Display, EnumString, EnumIter,
)]
#[strum(serialize_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum AiTaskKind {
    /// Evaluate how well a candidate fits a specific job posting.
    JobFit,
    /// Optimise a resume for a specific role or set of keywords.
    ResumeOptimize,
    /// Generate interview preparation materials.
    InterviewPrep,
    /// Draft a follow-up email after an interview or application.
    FollowUpDraft,
    /// Generate a cover letter tailored to a job posting.
    CoverLetter,
}

impl AiTaskKind {
    /// Return the default system prompt for this task kind.
    ///
    /// These are intentionally short placeholders; real prompts will be
    /// loaded from the `prompt_template` table via
    /// [`PromptTemplateManager`](crate::template::PromptTemplateManager).
    #[must_use]
    pub const fn default_system_prompt(&self) -> &'static str {
        match self {
            Self::JobFit => {
                "You are a career advisor. Analyze the job posting and the candidate's resume, \
                 then produce a structured fit score with reasoning."
            }
            Self::ResumeOptimize => {
                "You are a professional resume writer. Rewrite the resume to better match the \
                 target job description while keeping facts accurate."
            }
            Self::InterviewPrep => {
                "You are an interview coach. Generate likely interview questions and suggested \
                 answers based on the job description and resume."
            }
            Self::FollowUpDraft => {
                "You are a professional communicator. Draft a concise, polite follow-up email \
                 based on the context provided."
            }
            Self::CoverLetter => {
                "You are a professional cover-letter writer. Craft a compelling cover letter that \
                 highlights the candidate's relevant experience for the role."
            }
        }
    }

    /// Return an optional JSON schema describing the expected
    /// structured output for this task kind.
    ///
    /// Returns `None` for tasks that produce free-form text.
    #[must_use]
    pub fn default_output_schema(&self) -> Option<serde_json::Value> {
        match self {
            Self::JobFit => Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "fit_score": { "type": "number", "minimum": 0, "maximum": 100 },
                    "strengths": { "type": "array", "items": { "type": "string" } },
                    "gaps": { "type": "array", "items": { "type": "string" } },
                    "summary": { "type": "string" }
                },
                "required": ["fit_score", "strengths", "gaps", "summary"]
            })),
            Self::ResumeOptimize
            | Self::InterviewPrep
            | Self::FollowUpDraft
            | Self::CoverLetter => None,
        }
    }
}

/// Configuration for a specific AI task invocation.
///
/// Callers can override the default model, temperature, and other
/// parameters on a per-task basis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiTaskConfig {
    /// The kind of task to run.
    pub kind:           AiTaskKind,
    /// Override the model to use (otherwise the provider default is
    /// used).
    pub model_override: Option<String>,
    /// Override the sampling temperature.
    pub temperature:    Option<f32>,
    /// Override the maximum number of tokens to generate.
    pub max_tokens:     Option<u32>,
    /// Free-form key-value variables that will be substituted into the
    /// prompt template.
    pub variables:      HashMap<String, String>,
}

impl AiTaskConfig {
    /// Create a new task config with the given kind and variables.
    #[must_use]
    pub const fn new(kind: AiTaskKind, variables: HashMap<String, String>) -> Self {
        Self {
            kind,
            model_override: None,
            temperature: None,
            max_tokens: None,
            variables,
        }
    }
}
