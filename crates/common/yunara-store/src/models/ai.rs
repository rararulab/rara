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

//! AI-related entities: prompt templates and model run records.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// Category of a prompt template.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "prompt_kind", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum PromptKind {
    ResumeOptimize,
    CoverLetter,
    InterviewPrep,
    JobMatch,
    FollowUp,
    Other,
}

impl std::fmt::Display for PromptKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ResumeOptimize => write!(f, "resume_optimize"),
            Self::CoverLetter => write!(f, "cover_letter"),
            Self::InterviewPrep => write!(f, "interview_prep"),
            Self::JobMatch => write!(f, "job_match"),
            Self::FollowUp => write!(f, "follow_up"),
            Self::Other => write!(f, "other"),
        }
    }
}

/// AI model provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "ai_model_provider", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum AiModelProvider {
    Openai,
    Anthropic,
    Local,
    Other,
}

impl std::fmt::Display for AiModelProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Openai => write!(f, "openai"),
            Self::Anthropic => write!(f, "anthropic"),
            Self::Local => write!(f, "local"),
            Self::Other => write!(f, "other"),
        }
    }
}

/// A versioned prompt template used for AI model calls.
///
/// Templates are categorized by `kind` and versioned so that prompt
/// evolution can be tracked alongside AI run results.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct PromptTemplate {
    pub id:          Uuid,
    pub name:        String,
    pub kind:        PromptKind,
    pub version:     i32,
    pub content:     String,
    pub description: Option<String>,
    pub is_active:   bool,
    pub metadata:    Option<serde_json::Value>,
    pub trace_id:    Option<String>,
    pub is_deleted:  bool,
    pub deleted_at:  Option<DateTime<Utc>>,
    pub created_at:  DateTime<Utc>,
    pub updated_at:  DateTime<Utc>,
}

/// A record of a single AI model invocation.
///
/// Captures input/output summaries, token usage, cost, and latency
/// for observability and cost tracking.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AiRun {
    pub id:             Uuid,
    pub template_id:    Option<Uuid>,
    pub model_name:     String,
    pub provider:       AiModelProvider,
    pub input_summary:  Option<String>,
    pub output_summary: Option<String>,
    pub input_tokens:   i32,
    pub output_tokens:  i32,
    pub total_tokens:   i32,
    /// Cost in cents (integer to avoid floating-point issues).
    pub cost_cents:     i32,
    /// Wall-clock duration of the model call in milliseconds.
    pub duration_ms:    i32,
    pub is_success:     bool,
    pub error_message:  Option<String>,
    pub metadata:       Option<serde_json::Value>,
    pub trace_id:       Option<String>,
    pub created_at:     DateTime<Utc>,
}
