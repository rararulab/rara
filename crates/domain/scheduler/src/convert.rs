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

//! Conversion layer between DB (store) models and domain types for scheduler.

use job_domain_core::id::SchedulerTaskId;

use crate::db_models;
use crate::types;

// ===========================================================================
// TaskRunStatus conversions
// ===========================================================================

/// Store `TaskRunStatus` -> Domain `TaskRunStatus`.
impl From<db_models::TaskRunStatus> for types::TaskRunStatus {
    fn from(value: db_models::TaskRunStatus) -> Self {
        match value {
            db_models::TaskRunStatus::Success => Self::Success,
            db_models::TaskRunStatus::Failed => Self::Failed,
            db_models::TaskRunStatus::Running => Self::Running,
        }
    }
}

/// Domain `TaskRunStatus` -> Store `TaskRunStatus`.
impl From<types::TaskRunStatus> for db_models::TaskRunStatus {
    fn from(value: types::TaskRunStatus) -> Self {
        match value {
            types::TaskRunStatus::Success => Self::Success,
            types::TaskRunStatus::Failed => Self::Failed,
            types::TaskRunStatus::Running => Self::Running,
        }
    }
}

// ===========================================================================
// SchedulerTask <-> ScheduledTask conversions
// ===========================================================================

/// Store `SchedulerTask` -> Domain `ScheduledTask`.
impl From<db_models::SchedulerTask> for types::ScheduledTask {
    fn from(t: db_models::SchedulerTask) -> Self {
        Self {
            id:            SchedulerTaskId::from(t.id),
            name:          t.name,
            cron_expr:     t.cron_expr,
            enabled:       t.enabled,
            last_run_at:   t.last_run_at,
            last_status:   t.last_status.map(Into::into),
            last_error:    t.last_error,
            run_count:     t.run_count,
            failure_count: t.failure_count,
            created_at:    t.created_at,
            updated_at:    t.updated_at,
        }
    }
}

/// Domain `ScheduledTask` -> Store `SchedulerTask`.
impl From<types::ScheduledTask> for db_models::SchedulerTask {
    fn from(t: types::ScheduledTask) -> Self {
        Self {
            id:            t.id.into_inner(),
            name:          t.name,
            cron_expr:     t.cron_expr,
            enabled:       t.enabled,
            last_run_at:   t.last_run_at,
            last_status:   t.last_status.map(Into::into),
            last_error:    t.last_error,
            run_count:     t.run_count,
            failure_count: t.failure_count,
            is_deleted:    false,
            deleted_at:    None,
            created_at:    t.created_at,
            updated_at:    t.updated_at,
        }
    }
}

// ===========================================================================
// TaskRunHistory <-> TaskRunRecord conversions
// ===========================================================================

/// Store `TaskRunHistory` -> Domain `TaskRunRecord`.
impl From<db_models::TaskRunHistory> for types::TaskRunRecord {
    fn from(r: db_models::TaskRunHistory) -> Self {
        Self {
            id:          r.id,
            task_id:     SchedulerTaskId::from(r.task_id),
            status:      r.status.into(),
            started_at:  r.started_at,
            finished_at: r.finished_at,
            duration_ms: r.duration_ms,
            error:       r.error,
            output:      r.output,
            created_at:  r.created_at,
        }
    }
}

/// Domain `TaskRunRecord` -> Store `TaskRunHistory`.
impl From<types::TaskRunRecord> for db_models::TaskRunHistory {
    fn from(r: types::TaskRunRecord) -> Self {
        Self {
            id:          r.id,
            task_id:     r.task_id.into_inner(),
            status:      r.status.into(),
            started_at:  r.started_at,
            finished_at: r.finished_at,
            duration_ms: r.duration_ms,
            error:       r.error,
            output:      r.output,
            created_at:  r.created_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use uuid::Uuid;

    use super::*;

    #[test]
    fn task_run_status_roundtrip() {
        use db_models::TaskRunStatus as S;
        use types::TaskRunStatus as D;

        let pairs = [
            (S::Success, D::Success),
            (S::Failed, D::Failed),
            (S::Running, D::Running),
        ];
        for (store, domain) in &pairs {
            assert_eq!(D::from(*store), *domain);
            assert_eq!(S::from(*domain), *store);
        }
    }

    #[test]
    fn scheduler_task_store_to_domain_roundtrip() {
        let now = Utc::now();
        let id = Uuid::new_v4();
        let store_task = db_models::SchedulerTask {
            id,
            name: "job-discovery".into(),
            cron_expr: "0 */30 * * * *".into(),
            enabled: true,
            last_run_at: Some(now),
            last_status: Some(db_models::TaskRunStatus::Success),
            last_error: None,
            run_count: 5,
            failure_count: 1,
            is_deleted: false,
            deleted_at: None,
            created_at: now,
            updated_at: now,
        };

        let domain: types::ScheduledTask = store_task.into();
        assert_eq!(domain.id.into_inner(), id);
        assert_eq!(domain.name, "job-discovery");
        assert_eq!(domain.run_count, 5);
        assert_eq!(domain.last_status, Some(types::TaskRunStatus::Success));

        let back: db_models::SchedulerTask = domain.into();
        assert_eq!(back.id, id);
        assert_eq!(back.name, "job-discovery");
        assert!(!back.is_deleted);
    }

    #[test]
    fn task_run_history_store_to_domain_roundtrip() {
        let now = Utc::now();
        let id = Uuid::new_v4();
        let task_id = Uuid::new_v4();
        let store_run = db_models::TaskRunHistory {
            id,
            task_id,
            status: db_models::TaskRunStatus::Failed,
            started_at: now,
            finished_at: Some(now),
            duration_ms: Some(1500),
            error: Some("connection refused".into()),
            output: None,
            created_at: now,
        };

        let domain: types::TaskRunRecord = store_run.into();
        assert_eq!(domain.id, id);
        assert_eq!(domain.task_id.into_inner(), task_id);
        assert_eq!(domain.status, types::TaskRunStatus::Failed);
        assert_eq!(domain.duration_ms, Some(1500));

        let back: db_models::TaskRunHistory = domain.into();
        assert_eq!(back.id, id);
        assert_eq!(back.task_id, task_id);
        assert_eq!(back.error, Some("connection refused".to_owned()));
    }
}
