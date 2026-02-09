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

//! Conversion layer between DB models and domain types for interview.

use crate::db_models;
use crate::types;

// ---------------------------------------------------------------------------
// Interview round conversions
// ---------------------------------------------------------------------------

/// Parse a round string from the DB into a domain `InterviewRound`.
pub fn parse_interview_round(s: &str) -> types::InterviewRound {
    match s {
        "phone_screen" => types::InterviewRound::PhoneScreen,
        "technical" => types::InterviewRound::Technical,
        "system_design" => types::InterviewRound::SystemDesign,
        "behavioral" => types::InterviewRound::Behavioral,
        "culture_fit" => types::InterviewRound::CultureFit,
        "manager_round" => types::InterviewRound::ManagerRound,
        "final_round" => types::InterviewRound::FinalRound,
        other => types::InterviewRound::Other(other.to_owned()),
    }
}

/// Serialise a domain `InterviewRound` to its DB string representation.
pub fn interview_round_to_string(r: &types::InterviewRound) -> String {
    match r {
        types::InterviewRound::PhoneScreen => "phone_screen".to_owned(),
        types::InterviewRound::Technical => "technical".to_owned(),
        types::InterviewRound::SystemDesign => "system_design".to_owned(),
        types::InterviewRound::Behavioral => "behavioral".to_owned(),
        types::InterviewRound::CultureFit => "culture_fit".to_owned(),
        types::InterviewRound::ManagerRound => "manager_round".to_owned(),
        types::InterviewRound::FinalRound => "final_round".to_owned(),
        types::InterviewRound::Other(s) => s.clone(),
    }
}

// ---------------------------------------------------------------------------
// Interview task status conversions
// ---------------------------------------------------------------------------

/// Store `InterviewTaskStatus` -> Domain `InterviewTaskStatus`.
impl From<db_models::InterviewTaskStatus> for types::InterviewTaskStatus {
    fn from(s: db_models::InterviewTaskStatus) -> Self {
        match s {
            db_models::InterviewTaskStatus::Pending => Self::Pending,
            db_models::InterviewTaskStatus::InProgress => Self::InProgress,
            db_models::InterviewTaskStatus::Completed => Self::Completed,
            db_models::InterviewTaskStatus::Skipped => Self::Skipped,
        }
    }
}

/// Domain `InterviewTaskStatus` -> Store `InterviewTaskStatus`.
impl From<types::InterviewTaskStatus> for db_models::InterviewTaskStatus {
    fn from(s: types::InterviewTaskStatus) -> Self {
        match s {
            types::InterviewTaskStatus::Pending => Self::Pending,
            types::InterviewTaskStatus::InProgress => Self::InProgress,
            types::InterviewTaskStatus::Completed => Self::Completed,
            types::InterviewTaskStatus::Skipped => Self::Skipped,
        }
    }
}

// ---------------------------------------------------------------------------
// Interview plan conversions
// ---------------------------------------------------------------------------

/// Store `InterviewPlan` -> Domain `InterviewPlan`.
///
/// `materials` (JSONB) is deserialized into `PrepMaterials`; on failure
/// we fall back to `PrepMaterials::default()`.
impl From<db_models::InterviewPlan> for types::InterviewPlan {
    fn from(p: db_models::InterviewPlan) -> Self {
        let prep_materials: types::PrepMaterials = p
            .materials
            .as_ref()
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        Self {
            id:              job_domain_core::id::InterviewId::from(p.id),
            application_id:  job_domain_core::id::ApplicationId::from(p.application_id),
            title:           p.title,
            company:         p.company,
            position:        p.position,
            job_description: p.job_description,
            round:           parse_interview_round(&p.round),
            scheduled_at:    p.scheduled_at,
            task_status:     p.task_status.into(),
            prep_materials,
            notes:           p.notes,
            trace_id:        p.trace_id,
            is_deleted:      p.is_deleted,
            deleted_at:      p.deleted_at,
            created_at:      p.created_at,
            updated_at:      p.updated_at,
        }
    }
}

/// Domain `InterviewPlan` -> Store `InterviewPlan`.
///
/// `prep_materials` is serialised to JSONB.
impl From<types::InterviewPlan> for db_models::InterviewPlan {
    fn from(p: types::InterviewPlan) -> Self {
        let materials = serde_json::to_value(&p.prep_materials).ok();

        Self {
            id:              p.id.into_inner(),
            application_id:  p.application_id.into_inner(),
            title:           p.title,
            company:         p.company,
            position:        p.position,
            job_description: p.job_description,
            round:           interview_round_to_string(&p.round),
            description:     None,
            scheduled_at:    p.scheduled_at,
            task_status:     p.task_status.into(),
            materials,
            notes:           p.notes,
            trace_id:        p.trace_id,
            is_deleted:      p.is_deleted,
            deleted_at:      p.deleted_at,
            created_at:      p.created_at,
            updated_at:      p.updated_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    #[test]
    fn interview_round_parse_and_serialize() {
        let cases = vec![
            (types::InterviewRound::PhoneScreen, "phone_screen"),
            (types::InterviewRound::Technical, "technical"),
            (types::InterviewRound::SystemDesign, "system_design"),
            (types::InterviewRound::Behavioral, "behavioral"),
            (types::InterviewRound::CultureFit, "culture_fit"),
            (types::InterviewRound::ManagerRound, "manager_round"),
            (types::InterviewRound::FinalRound, "final_round"),
        ];
        for (round, expected) in cases {
            let s = interview_round_to_string(&round);
            assert_eq!(s, expected);
            assert_eq!(parse_interview_round(&s), round);
        }
        // Custom round
        assert_eq!(
            parse_interview_round("panel"),
            types::InterviewRound::Other("panel".to_owned())
        );
    }

    #[test]
    fn interview_task_status_roundtrip() {
        use db_models::InterviewTaskStatus as S;
        use types::InterviewTaskStatus as D;

        let pairs = [
            (S::Pending, D::Pending),
            (S::InProgress, D::InProgress),
            (S::Completed, D::Completed),
            (S::Skipped, D::Skipped),
        ];
        for (store, domain) in &pairs {
            assert_eq!(D::from(*store), *domain);
            assert_eq!(S::from(*domain), *store);
        }
    }

    #[test]
    fn interview_plan_store_to_domain() {
        let now = Utc::now();
        let materials = serde_json::json!({
            "knowledge_points": ["Rust", "async"],
            "project_review_items": [],
            "behavioral_questions": [],
            "questions_to_ask": [],
            "additional_resources": []
        });

        let store = db_models::InterviewPlan {
            id: Uuid::new_v4(),
            application_id: Uuid::new_v4(),
            title: "Tech Screen".into(),
            company: "Acme".into(),
            position: "SWE".into(),
            job_description: Some("Build stuff".into()),
            round: "technical".into(),
            description: None,
            scheduled_at: None,
            task_status: db_models::InterviewTaskStatus::Pending,
            materials: Some(materials),
            notes: None,
            trace_id: None,
            is_deleted: false,
            deleted_at: None,
            created_at: now,
            updated_at: now,
        };

        let domain: types::InterviewPlan = store.into();
        assert_eq!(domain.company, "Acme");
        assert_eq!(domain.round, types::InterviewRound::Technical);
        assert_eq!(domain.prep_materials.knowledge_points.len(), 2);
    }

    #[test]
    fn interview_plan_domain_to_store() {
        let now = Utc::now();
        let domain = types::InterviewPlan {
            id: job_domain_core::id::InterviewId::from(Uuid::new_v4()),
            application_id: job_domain_core::id::ApplicationId::from(Uuid::new_v4()),
            title: "Final".into(),
            company: "BigCo".into(),
            position: "Staff".into(),
            job_description: None,
            round: types::InterviewRound::FinalRound,
            scheduled_at: None,
            task_status: types::InterviewTaskStatus::Completed,
            prep_materials: types::PrepMaterials::default(),
            notes: Some("Went well".into()),
            trace_id: None,
            is_deleted: false,
            deleted_at: None,
            created_at: now,
            updated_at: now,
        };

        let store: db_models::InterviewPlan = domain.into();
        assert_eq!(store.company, "BigCo");
        assert_eq!(store.round, "final_round");
        assert_eq!(store.task_status, db_models::InterviewTaskStatus::Completed);
        assert!(store.materials.is_some());
    }

    #[test]
    fn interview_plan_null_materials_defaults() {
        let now = Utc::now();
        let store = db_models::InterviewPlan {
            id: Uuid::new_v4(),
            application_id: Uuid::new_v4(),
            title: "Screen".into(),
            company: "".into(),
            position: "".into(),
            job_description: None,
            round: "phone_screen".into(),
            description: None,
            scheduled_at: None,
            task_status: db_models::InterviewTaskStatus::Pending,
            materials: None,
            notes: None,
            trace_id: None,
            is_deleted: false,
            deleted_at: None,
            created_at: now,
            updated_at: now,
        };

        let domain: types::InterviewPlan = store.into();
        assert!(domain.prep_materials.knowledge_points.is_empty());
    }
}
