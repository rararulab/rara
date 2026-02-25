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

//! Trait (port) for AI-powered interview prep generation.
//!
//! The interview domain defines this trait so that it does **not** depend
//! on the AI crate directly.  The AI module (or any other adapter)
//! provides a concrete implementation.

use async_trait::async_trait;

use super::{
    error::InterviewError,
    types::{
        BehavioralQuestion, InterviewRound, PrepGenerationRequest, PrepMaterials, ProjectReview,
    },
};

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Generate interview preparation materials from a request.
///
/// Implementations may call an LLM, a template engine, or simply
/// return hard-coded data (see [`MockPrepGenerator`]).
#[async_trait]
pub trait PrepGenerator: Send + Sync {
    /// Produce [`PrepMaterials`] for the given request.
    async fn generate_prep(
        &self,
        request: &PrepGenerationRequest,
    ) -> Result<PrepMaterials, InterviewError>;
}

// ---------------------------------------------------------------------------
// Mock implementation
// ---------------------------------------------------------------------------

/// A deterministic mock that returns template prep materials.
///
/// Useful for unit tests and local development without an AI backend.
pub struct MockPrepGenerator;

impl MockPrepGenerator {
    /// Create a new mock generator.
    #[must_use]
    pub const fn new() -> Self { Self }
}

impl Default for MockPrepGenerator {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl PrepGenerator for MockPrepGenerator {
    async fn generate_prep(
        &self,
        request: &PrepGenerationRequest,
    ) -> Result<PrepMaterials, InterviewError> {
        let knowledge_points = match request.round {
            InterviewRound::Technical | InterviewRound::SystemDesign => vec![
                format!(
                    "Core technical skills for {} at {}",
                    request.position, request.company
                ),
                "Data structures and algorithms".into(),
                "System design fundamentals".into(),
                "Language-specific best practices".into(),
            ],
            InterviewRound::Behavioral | InterviewRound::CultureFit => vec![
                "Leadership principles".into(),
                "Conflict resolution".into(),
                "Team collaboration".into(),
            ],
            _ => vec![
                format!(
                    "General preparation for {} role at {}",
                    request.position, request.company
                ),
                "Company research".into(),
                "Role-specific knowledge".into(),
            ],
        };

        let project_review_items = vec![ProjectReview {
            project_name:        "Relevant Project".into(),
            key_points:          vec![
                "Technical decisions and trade-offs".into(),
                "Impact and measurable outcomes".into(),
            ],
            potential_questions: vec![
                "What was your specific contribution?".into(),
                "What would you do differently?".into(),
            ],
        }];

        let behavioral_questions = vec![
            BehavioralQuestion {
                question:           "Tell me about a challenging project you led.".into(),
                suggested_approach: "Use STAR method: Situation, Task, Action, Result".into(),
                example_situation:  "Describe a project with tight deadlines or technical \
                                     complexity."
                    .into(),
            },
            BehavioralQuestion {
                question:           "Describe a time you disagreed with a teammate.".into(),
                suggested_approach: "Focus on collaboration and resolution".into(),
                example_situation:  "Highlight a technical disagreement that led to a better \
                                     outcome."
                    .into(),
            },
        ];

        let questions_to_ask = vec![
            format!(
                "What does a typical day look like for this {} role?",
                request.position
            ),
            format!(
                "What are the biggest challenges facing the team at {}?",
                request.company
            ),
            "How do you measure success in this role?".into(),
            "What does the onboarding process look like?".into(),
        ];

        let additional_resources = vec![
            format!(
                "https://www.glassdoor.com/Reviews/{}-Reviews",
                request.company
            ),
            "Review the job description key requirements".into(),
        ];

        Ok(PrepMaterials {
            knowledge_points,
            project_review_items,
            behavioral_questions,
            questions_to_ask,
            additional_resources,
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request(round: InterviewRound) -> PrepGenerationRequest {
        PrepGenerationRequest {
            company: "Acme Corp".into(),
            position: "Senior Rust Engineer".into(),
            job_description: "Build distributed systems in Rust.".into(),
            round,
            resume_content: None,
            previous_rounds: vec![],
            email_context: None,
        }
    }

    #[tokio::test]
    async fn mock_generates_technical_prep() {
        let generator = MockPrepGenerator::new();
        let req = sample_request(InterviewRound::Technical);
        let materials = generator.generate_prep(&req).await.expect("should succeed");

        assert!(
            !materials.knowledge_points.is_empty(),
            "should have knowledge points"
        );
        assert!(
            materials.knowledge_points[0].contains("Senior Rust Engineer"),
            "first point should reference position"
        );
        assert!(
            !materials.behavioral_questions.is_empty(),
            "should have behavioral questions"
        );
        assert!(
            !materials.questions_to_ask.is_empty(),
            "should have questions to ask"
        );
    }

    #[tokio::test]
    async fn mock_generates_behavioral_prep() {
        let generator = MockPrepGenerator::new();
        let req = sample_request(InterviewRound::Behavioral);
        let materials = generator.generate_prep(&req).await.expect("should succeed");

        assert!(
            materials
                .knowledge_points
                .iter()
                .any(|kp| kp.contains("Leadership")),
            "behavioral prep should mention leadership"
        );
    }

    #[tokio::test]
    async fn mock_generates_generic_prep_for_phone_screen() {
        let generator = MockPrepGenerator::new();
        let req = sample_request(InterviewRound::PhoneScreen);
        let materials = generator.generate_prep(&req).await.expect("should succeed");

        assert!(
            materials
                .knowledge_points
                .iter()
                .any(|kp| kp.contains("General preparation")),
            "phone screen should get general prep"
        );
    }
}
