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

use chrono::{DateTime, TimeZone as _, Utc};
use jiff::Timestamp;

use crate::types;
use job_model::interview::InterviewPlan as StoreInterviewPlan;

fn chrono_to_timestamp(dt: DateTime<Utc>) -> Timestamp {
    Timestamp::new(dt.timestamp(), dt.timestamp_subsec_nanos() as i32)
        .expect("chrono DateTime<Utc> fits in jiff Timestamp")
}

fn chrono_opt_to_timestamp(dt: Option<DateTime<Utc>>) -> Option<Timestamp> {
    dt.map(chrono_to_timestamp)
}

fn timestamp_to_chrono(ts: Timestamp) -> DateTime<Utc> {
    let mut second = ts.as_second();
    let mut nanosecond = ts.subsec_nanosecond();
    if nanosecond < 0 {
        second = second.saturating_sub(1);
        nanosecond = nanosecond.saturating_add(1_000_000_000);
    }

    Utc.timestamp_opt(second, nanosecond as u32)
        .single()
        .expect("jiff Timestamp fits in chrono DateTime<Utc>")
}

fn timestamp_opt_to_chrono(ts: Option<Timestamp>) -> Option<DateTime<Utc>> {
    ts.map(timestamp_to_chrono)
}

fn u8_from_i16(value: i16, field: &'static str) -> u8 {
    u8::try_from(value).unwrap_or_else(|_| panic!("invalid {field}: {value}"))
}

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

fn interview_task_status_from_i16(value: i16) -> types::InterviewTaskStatus {
    let repr = u8_from_i16(value, "interview_plan.task_status");
    types::InterviewTaskStatus::from_repr(repr)
        .unwrap_or_else(|| panic!("invalid interview_plan.task_status: {value}"))
}

// ---------------------------------------------------------------------------
// Interview plan conversions
// ---------------------------------------------------------------------------

/// Store `InterviewPlan` -> Domain `InterviewPlan`.
///
/// `materials` (JSONB) is deserialized into `PrepMaterials`; on failure
/// we fall back to `PrepMaterials::default()`.
impl From<StoreInterviewPlan> for types::InterviewPlan {
    fn from(p: StoreInterviewPlan) -> Self {
        let prep_materials: types::PrepMaterials = p
            .materials
            .as_ref()
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        Self {
            id: job_domain_shared::id::InterviewId::from(p.id),
            application_id: job_domain_shared::id::ApplicationId::from(p.application_id),
            title: p.title,
            company: p.company,
            position: p.position,
            job_description: p.job_description,
            round: parse_interview_round(&p.round),
            scheduled_at: chrono_opt_to_timestamp(p.scheduled_at),
            task_status: interview_task_status_from_i16(p.task_status),
            prep_materials,
            notes: p.notes,
            trace_id: p.trace_id,
            is_deleted: p.is_deleted,
            deleted_at: chrono_opt_to_timestamp(p.deleted_at),
            created_at: chrono_to_timestamp(p.created_at),
            updated_at: chrono_to_timestamp(p.updated_at),
        }
    }
}

/// Domain `InterviewPlan` -> Store `InterviewPlan`.
///
/// `prep_materials` is serialised to JSONB.
impl From<types::InterviewPlan> for StoreInterviewPlan {
    fn from(p: types::InterviewPlan) -> Self {
        let materials = serde_json::to_value(&p.prep_materials).ok();

        Self {
            id: p.id.into_inner(),
            application_id: p.application_id.into_inner(),
            title: p.title,
            company: p.company,
            position: p.position,
            job_description: p.job_description,
            round: interview_round_to_string(&p.round),
            description: None,
            scheduled_at: timestamp_opt_to_chrono(p.scheduled_at),
            task_status: p.task_status as u8 as i16,
            materials,
            notes: p.notes,
            trace_id: p.trace_id,
            is_deleted: p.is_deleted,
            deleted_at: timestamp_opt_to_chrono(p.deleted_at),
            created_at: timestamp_to_chrono(p.created_at),
            updated_at: timestamp_to_chrono(p.updated_at),
        }
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

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
    fn interview_task_status_from_i16_works() {
        use types::InterviewTaskStatus as D;

        assert_eq!(interview_task_status_from_i16(0), D::Pending);
        assert_eq!(interview_task_status_from_i16(1), D::InProgress);
        assert_eq!(interview_task_status_from_i16(2), D::Completed);
        assert_eq!(interview_task_status_from_i16(3), D::Skipped);
    }

    #[test]
    fn interview_plan_store_to_domain() {
        let now = chrono::Utc::now();
        let materials = serde_json::json!({
            "knowledge_points": ["Rust", "async"],
            "project_review_items": [],
            "behavioral_questions": [],
            "questions_to_ask": [],
            "additional_resources": []
        });

        let store = StoreInterviewPlan {
            id:              Uuid::new_v4(),
            application_id:  Uuid::new_v4(),
            title:           "Tech Screen".into(),
            company:         "Acme".into(),
            position:        "SWE".into(),
            job_description: Some("Build stuff".into()),
            round:           "technical".into(),
            description:     None,
            scheduled_at:    None,
            task_status:     0,
            materials:       Some(materials),
            notes:           None,
            trace_id:        None,
            is_deleted:      false,
            deleted_at:      None,
            created_at:      now,
            updated_at:      now,
        };

        let domain: types::InterviewPlan = store.into();
        assert_eq!(domain.company, "Acme");
        assert_eq!(domain.round, types::InterviewRound::Technical);
        assert_eq!(domain.prep_materials.knowledge_points.len(), 2);
    }

    #[test]
    fn interview_plan_domain_to_store() {
        let now = jiff::Timestamp::now();
        let domain = types::InterviewPlan {
            id:              job_domain_shared::id::InterviewId::from(Uuid::new_v4()),
            application_id:  job_domain_shared::id::ApplicationId::from(Uuid::new_v4()),
            title:           "Final".into(),
            company:         "BigCo".into(),
            position:        "Staff".into(),
            job_description: None,
            round:           types::InterviewRound::FinalRound,
            scheduled_at:    None,
            task_status:     types::InterviewTaskStatus::Completed,
            prep_materials:  types::PrepMaterials::default(),
            notes:           Some("Went well".into()),
            trace_id:        None,
            is_deleted:      false,
            deleted_at:      None,
            created_at:      now,
            updated_at:      now,
        };

        let store: StoreInterviewPlan = domain.into();
        assert_eq!(store.company, "BigCo");
        assert_eq!(store.round, "final_round");
        assert_eq!(store.task_status, 2);
        assert!(store.materials.is_some());
    }

    #[test]
    fn interview_plan_null_materials_defaults() {
        let now = chrono::Utc::now();
        let store = StoreInterviewPlan {
            id:              Uuid::new_v4(),
            application_id:  Uuid::new_v4(),
            title:           "Screen".into(),
            company:         "".into(),
            position:        "".into(),
            job_description: None,
            round:           "phone_screen".into(),
            description:     None,
            scheduled_at:    None,
            task_status:     0,
            materials:       None,
            notes:           None,
            trace_id:        None,
            is_deleted:      false,
            deleted_at:      None,
            created_at:      now,
            updated_at:      now,
        };

        let domain: types::InterviewPlan = store.into();
        assert!(domain.prep_materials.knowledge_points.is_empty());
    }
}
