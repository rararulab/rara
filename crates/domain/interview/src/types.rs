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

//! Domain types for interview plan management.

use jiff::Timestamp;
use job_domain_core::id::{ApplicationId, InterviewId};
use serde::{Deserialize, Serialize};
use strum_macros::{Display, FromRepr};

// ---------------------------------------------------------------------------
// Interview round
// ---------------------------------------------------------------------------

/// Which round of the interview process this plan covers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterviewRound {
    /// Initial phone/video screening.
    PhoneScreen,
    /// Technical / coding interview.
    Technical,
    /// System design interview.
    SystemDesign,
    /// Behavioral interview.
    Behavioral,
    /// Culture-fit assessment.
    CultureFit,
    /// Hiring-manager round.
    ManagerRound,
    /// Final / decision round.
    FinalRound,
    /// Any other round type.
    Other(String),
}

impl std::fmt::Display for InterviewRound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PhoneScreen => write!(f, "phone_screen"),
            Self::Technical => write!(f, "technical"),
            Self::SystemDesign => write!(f, "system_design"),
            Self::Behavioral => write!(f, "behavioral"),
            Self::CultureFit => write!(f, "culture_fit"),
            Self::ManagerRound => write!(f, "manager_round"),
            Self::FinalRound => write!(f, "final_round"),
            Self::Other(s) => write!(f, "other({s})"),
        }
    }
}

// ---------------------------------------------------------------------------
// Interview task status
// ---------------------------------------------------------------------------

/// Task-level status of an interview plan (maps to the DB enum
/// `interview_task_status`).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, FromRepr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum InterviewTaskStatus {
    /// Plan created but not yet started.
    Pending = 0,
    /// Actively working on the prep.
    InProgress = 1,
    /// Preparation completed.
    Completed = 2,
    /// Skipped (e.g. interview cancelled before prep finished).
    Skipped = 3,
}

// ---------------------------------------------------------------------------
// Prep materials (JSONB)
// ---------------------------------------------------------------------------

/// AI-generated (or manually curated) preparation materials for an
/// interview.
///
/// Serialised as JSONB in the `materials` column of `interview_plan`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PrepMaterials {
    /// Key knowledge points to review.
    pub knowledge_points:     Vec<String>,
    /// Projects from the candidate's background to review.
    pub project_review_items: Vec<ProjectReview>,
    /// Behavioural questions with suggested answers.
    pub behavioral_questions: Vec<BehavioralQuestion>,
    /// Questions the candidate should ask the interviewer.
    pub questions_to_ask:     Vec<String>,
    /// Links, articles, or other reference material.
    pub additional_resources: Vec<String>,
}

/// A single project-review entry inside [`PrepMaterials`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectReview {
    /// Name of the project.
    pub project_name:        String,
    /// Key talking points.
    pub key_points:          Vec<String>,
    /// Questions an interviewer might ask about this project.
    pub potential_questions: Vec<String>,
}

/// A single behavioural question inside [`PrepMaterials`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehavioralQuestion {
    /// The question itself.
    pub question:           String,
    /// A suggested answering approach (e.g. STAR method outline).
    pub suggested_approach: String,
    /// An example situation the candidate could use.
    pub example_situation:  String,
}

// ---------------------------------------------------------------------------
// Interview plan (aggregate root)
// ---------------------------------------------------------------------------

/// An interview preparation plan linked to a specific application.
///
/// This is the primary aggregate root for the interview domain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterviewPlan {
    /// Unique identifier.
    pub id:              InterviewId,
    /// The application this interview belongs to.
    pub application_id:  ApplicationId,
    /// Short human-readable title.
    pub title:           String,
    /// Target company name.
    pub company:         String,
    /// Target position title.
    pub position:        String,
    /// Full job description text.
    pub job_description: Option<String>,
    /// Which interview round this plan covers.
    pub round:           InterviewRound,
    /// Scheduled date/time of the interview.
    pub scheduled_at:    Option<Timestamp>,
    /// Current task status.
    pub task_status:     InterviewTaskStatus,
    /// AI-generated or manually curated prep materials.
    pub prep_materials:  PrepMaterials,
    /// Free-form notes.
    pub notes:           Option<String>,
    /// Distributed tracing correlation id.
    pub trace_id:        Option<String>,
    /// Soft-delete flag.
    pub is_deleted:      bool,
    /// When the record was soft-deleted.
    pub deleted_at:      Option<Timestamp>,
    /// When the record was created.
    pub created_at:      Timestamp,
    /// When the record was last updated.
    pub updated_at:      Timestamp,
}

// ---------------------------------------------------------------------------
// Request / filter DTOs
// ---------------------------------------------------------------------------

/// Parameters for creating a new interview plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateInterviewPlanRequest {
    /// The application this interview belongs to.
    pub application_id:  ApplicationId,
    /// Short title for the plan.
    pub title:           String,
    /// Company name.
    pub company:         String,
    /// Position title.
    pub position:        String,
    /// Full job description.
    pub job_description: Option<String>,
    /// Interview round.
    pub round:           InterviewRound,
    /// Scheduled date/time.
    pub scheduled_at:    Option<Timestamp>,
    /// Optional free-form notes.
    pub notes:           Option<String>,
}

/// Parameters for a partial update to an existing interview plan.
///
/// Only fields set to `Some` will be applied.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateInterviewPlanRequest {
    /// New title.
    pub title:           Option<String>,
    /// New company name.
    pub company:         Option<String>,
    /// New position.
    pub position:        Option<String>,
    /// New job description.
    pub job_description: Option<Option<String>>,
    /// New round.
    pub round:           Option<InterviewRound>,
    /// New scheduled time.
    pub scheduled_at:    Option<Option<Timestamp>>,
    /// Replace prep materials entirely.
    pub prep_materials:  Option<PrepMaterials>,
    /// New notes.
    pub notes:           Option<Option<String>>,
}

/// Criteria for listing/searching interview plans.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InterviewFilter {
    /// Filter by application.
    pub application_id:   Option<ApplicationId>,
    /// Filter by company name (exact match).
    pub company:          Option<String>,
    /// Filter by task status.
    pub task_status:      Option<InterviewTaskStatus>,
    /// Filter by interview round.
    pub round:            Option<InterviewRound>,
    /// Scheduled at or after this timestamp.
    pub scheduled_after:  Option<Timestamp>,
    /// Scheduled at or before this timestamp.
    pub scheduled_before: Option<Timestamp>,
}

/// Input for AI prep-material generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrepGenerationRequest {
    /// Target company.
    pub company:         String,
    /// Target position.
    pub position:        String,
    /// Full job description text.
    pub job_description: String,
    /// Which interview round to prepare for.
    pub round:           InterviewRound,
    /// Candidate's resume content (if available).
    pub resume_content:  Option<String>,
    /// Summaries of previous interview rounds.
    pub previous_rounds: Vec<String>,
    /// Relevant email / communication context.
    pub email_context:   Option<String>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interview_round_display() {
        assert_eq!(InterviewRound::PhoneScreen.to_string(), "phone_screen");
        assert_eq!(InterviewRound::Technical.to_string(), "technical");
        assert_eq!(InterviewRound::SystemDesign.to_string(), "system_design");
        assert_eq!(
            InterviewRound::Other("panel".into()).to_string(),
            "other(panel)"
        );
    }

    #[test]
    fn interview_task_status_display() {
        assert_eq!(InterviewTaskStatus::Pending.to_string(), "pending");
        assert_eq!(InterviewTaskStatus::InProgress.to_string(), "in_progress");
        assert_eq!(InterviewTaskStatus::Completed.to_string(), "completed");
        assert_eq!(InterviewTaskStatus::Skipped.to_string(), "skipped");
    }

    #[test]
    fn prep_materials_serde_roundtrip() {
        let materials = PrepMaterials {
            knowledge_points:     vec!["Rust ownership".into(), "async/await".into()],
            project_review_items: vec![ProjectReview {
                project_name:        "My API".into(),
                key_points:          vec!["Used Axum".into()],
                potential_questions: vec!["How did you handle errors?".into()],
            }],
            behavioral_questions: vec![BehavioralQuestion {
                question:           "Tell me about a time you led a project.".into(),
                suggested_approach: "Use STAR method".into(),
                example_situation:  "Led migration to microservices".into(),
            }],
            questions_to_ask:     vec!["What does the team look like?".into()],
            additional_resources: vec!["https://example.com/rust-book".into()],
        };

        let json = serde_json::to_string(&materials).expect("serialize");
        let deserialized: PrepMaterials = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(deserialized.knowledge_points.len(), 2);
        assert_eq!(deserialized.project_review_items.len(), 1);
        assert_eq!(deserialized.behavioral_questions.len(), 1);
        assert_eq!(deserialized.questions_to_ask.len(), 1);
        assert_eq!(deserialized.additional_resources.len(), 1);
    }

    #[test]
    fn interview_round_serde_roundtrip() {
        let rounds = vec![
            InterviewRound::PhoneScreen,
            InterviewRound::Technical,
            InterviewRound::Other("custom".into()),
        ];

        let json = serde_json::to_string(&rounds).expect("serialize");
        let deserialized: Vec<InterviewRound> = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(deserialized, rounds);
    }
}
