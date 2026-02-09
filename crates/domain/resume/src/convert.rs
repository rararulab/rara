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

use crate::db_models;
use crate::types;

// ---------------------------------------------------------------------------
// ResumeSource conversions
// ---------------------------------------------------------------------------

/// Store `ResumeSource` -> Domain `ResumeSource`.
impl From<db_models::ResumeSource> for types::ResumeSource {
    fn from(s: db_models::ResumeSource) -> Self {
        match s {
            db_models::ResumeSource::Manual => Self::Manual,
            db_models::ResumeSource::AiGenerated => Self::AiGenerated,
            db_models::ResumeSource::Optimized => Self::Optimized,
        }
    }
}

/// Domain `ResumeSource` -> Store `ResumeSource`.
impl From<types::ResumeSource> for db_models::ResumeSource {
    fn from(s: types::ResumeSource) -> Self {
        match s {
            types::ResumeSource::Manual => Self::Manual,
            types::ResumeSource::AiGenerated => Self::AiGenerated,
            types::ResumeSource::Optimized => Self::Optimized,
        }
    }
}

// ---------------------------------------------------------------------------
// Resume conversions
// ---------------------------------------------------------------------------

/// Store `Resume` -> Domain `Resume`.
impl From<db_models::Resume> for types::Resume {
    fn from(r: db_models::Resume) -> Self {
        Self {
            id:                  r.id,
            title:               r.title,
            version_tag:         r.version_tag,
            content_hash:        r.content_hash,
            source:              r.source.into(),
            content:             r.content,
            parent_resume_id:    r.parent_resume_id,
            target_job_id:       r.target_job_id,
            customization_notes: r.customization_notes,
            tags:                r.tags,
            metadata:            r.metadata,
            trace_id:            r.trace_id,
            is_deleted:          r.is_deleted,
            deleted_at:          r.deleted_at,
            created_at:          r.created_at,
            updated_at:          r.updated_at,
        }
    }
}

/// Domain `Resume` -> Store `Resume`.
impl From<types::Resume> for db_models::Resume {
    fn from(r: types::Resume) -> Self {
        Self {
            id:                  r.id,
            title:               r.title,
            version_tag:         r.version_tag,
            content_hash:        r.content_hash,
            source:              r.source.into(),
            content:             r.content,
            parent_resume_id:    r.parent_resume_id,
            target_job_id:       r.target_job_id,
            customization_notes: r.customization_notes,
            tags:                r.tags,
            metadata:            r.metadata,
            trace_id:            r.trace_id,
            is_deleted:          r.is_deleted,
            deleted_at:          r.deleted_at,
            created_at:          r.created_at,
            updated_at:          r.updated_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use uuid::Uuid;

    use super::*;

    #[test]
    fn resume_source_roundtrip() {
        let pairs = [
            (db_models::ResumeSource::Manual, types::ResumeSource::Manual),
            (
                db_models::ResumeSource::AiGenerated,
                types::ResumeSource::AiGenerated,
            ),
            (
                db_models::ResumeSource::Optimized,
                types::ResumeSource::Optimized,
            ),
        ];
        for (store, domain) in &pairs {
            assert_eq!(types::ResumeSource::from(*store), *domain);
            assert_eq!(db_models::ResumeSource::from(*domain), *store);
        }
    }

    #[test]
    fn resume_store_to_domain_roundtrip() {
        let now = Utc::now();
        let id = Uuid::new_v4();
        let store_resume = db_models::Resume {
            id,
            title: "Backend v1".into(),
            version_tag: "v1.0".into(),
            content_hash: "abc123".into(),
            source: db_models::ResumeSource::Manual,
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

        let back: db_models::Resume = domain.into();
        assert_eq!(back.id, id);
        assert_eq!(back.title, "Backend v1");
    }
}
