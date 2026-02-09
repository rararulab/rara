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

//! Conversion layer between DB models and domain types for resume.

use chrono::{DateTime, TimeZone as _, Utc};
use jiff::Timestamp;

use crate::types;
use job_model::resume::Resume as StoreResume;

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

fn resume_source_from_i16(value: i16) -> types::ResumeSource {
    let repr = u8_from_i16(value, "resume.source");
    types::ResumeSource::from_repr(repr).unwrap_or_else(|| panic!("invalid resume.source: {value}"))
}

// ---------------------------------------------------------------------------
// Resume conversions
// ---------------------------------------------------------------------------

/// Store `Resume` -> Domain `Resume`.
impl From<StoreResume> for types::Resume {
    fn from(r: StoreResume) -> Self {
        Self {
            id:                  r.id,
            title:               r.title,
            version_tag:         r.version_tag,
            content_hash:        r.content_hash,
            source:              resume_source_from_i16(r.source),
            content:             r.content,
            parent_resume_id:    r.parent_resume_id,
            target_job_id:       r.target_job_id,
            customization_notes: r.customization_notes,
            tags:                r.tags,
            metadata:            r.metadata,
            trace_id:            r.trace_id,
            is_deleted:          r.is_deleted,
            deleted_at:          chrono_opt_to_timestamp(r.deleted_at),
            created_at:          chrono_to_timestamp(r.created_at),
            updated_at:          chrono_to_timestamp(r.updated_at),
        }
    }
}

/// Domain `Resume` -> Store `Resume`.
impl From<types::Resume> for StoreResume {
    fn from(r: types::Resume) -> Self {
        Self {
            id:                  r.id,
            title:               r.title,
            version_tag:         r.version_tag,
            content_hash:        r.content_hash,
            source:              r.source as u8 as i16,
            content:             r.content,
            parent_resume_id:    r.parent_resume_id,
            target_job_id:       r.target_job_id,
            customization_notes: r.customization_notes,
            tags:                r.tags,
            metadata:            r.metadata,
            trace_id:            r.trace_id,
            is_deleted:          r.is_deleted,
            deleted_at:          timestamp_opt_to_chrono(r.deleted_at),
            created_at:          timestamp_to_chrono(r.created_at),
            updated_at:          timestamp_to_chrono(r.updated_at),
        }
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    #[test]
    fn resume_source_from_i16_works() {
        use types::ResumeSource as D;

        assert_eq!(resume_source_from_i16(0), D::Manual);
        assert_eq!(resume_source_from_i16(1), D::AiGenerated);
        assert_eq!(resume_source_from_i16(2), D::Optimized);
    }

    #[test]
    fn resume_store_to_domain_roundtrip() {
        let now = chrono::Utc::now();
        let id = Uuid::new_v4();
        let store_resume = StoreResume {
            id,
            title: "Backend v1".into(),
            version_tag: "v1.0".into(),
            content_hash: "abc123".into(),
            source: 0,
            content: Some("Resume content".into()),
            parent_resume_id: None,
            target_job_id: None,
            customization_notes: None,
            tags: vec!["rust".into()],
            metadata: None,
            trace_id: None,
            is_deleted: false,
            deleted_at: None,
            created_at: now,
            updated_at: now,
        };

        let domain: types::Resume = store_resume.into();
        assert_eq!(domain.id, id);
        assert_eq!(domain.title, "Backend v1");
        assert_eq!(domain.tags, vec!["rust".to_owned()]);

        let back: StoreResume = domain.into();
        assert_eq!(back.id, id);
        assert_eq!(back.title, "Backend v1");
    }
}
