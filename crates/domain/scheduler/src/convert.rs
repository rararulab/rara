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

use chrono::{DateTime, TimeZone as _, Utc};
use jiff::Timestamp;
use job_domain_shared::id::SchedulerTaskId;

use crate::{db_models, types};

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

fn task_run_status_from_i16(value: i16) -> types::TaskRunStatus {
    let repr = u8_from_i16(value, "scheduler_task.last_status/task_run_history.status");
    types::TaskRunStatus::from_repr(repr)
        .unwrap_or_else(|| panic!("invalid task run status: {value}"))
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
            last_run_at:   chrono_opt_to_timestamp(t.last_run_at),
            last_status:   t.last_status.map(task_run_status_from_i16),
            last_error:    t.last_error,
            run_count:     t.run_count,
            failure_count: t.failure_count,
            created_at:    chrono_to_timestamp(t.created_at),
            updated_at:    chrono_to_timestamp(t.updated_at),
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
            last_run_at:   timestamp_opt_to_chrono(t.last_run_at),
            last_status:   t.last_status.map(|s| s as u8 as i16),
            last_error:    t.last_error,
            run_count:     t.run_count,
            failure_count: t.failure_count,
            is_deleted:    false,
            deleted_at:    None,
            created_at:    timestamp_to_chrono(t.created_at),
            updated_at:    timestamp_to_chrono(t.updated_at),
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
            status:      task_run_status_from_i16(r.status),
            started_at:  chrono_to_timestamp(r.started_at),
            finished_at: chrono_opt_to_timestamp(r.finished_at),
            duration_ms: r.duration_ms,
            error:       r.error,
            output:      r.output,
            created_at:  chrono_to_timestamp(r.created_at),
        }
    }
}

/// Domain `TaskRunRecord` -> Store `TaskRunHistory`.
impl From<types::TaskRunRecord> for db_models::TaskRunHistory {
    fn from(r: types::TaskRunRecord) -> Self {
        Self {
            id:          r.id,
            task_id:     r.task_id.into_inner(),
            status:      r.status as u8 as i16,
            started_at:  timestamp_to_chrono(r.started_at),
            finished_at: timestamp_opt_to_chrono(r.finished_at),
            duration_ms: r.duration_ms,
            error:       r.error,
            output:      r.output,
            created_at:  timestamp_to_chrono(r.created_at),
        }
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    #[test]
    fn task_run_status_from_i16_works() {
        use types::TaskRunStatus as D;

        assert_eq!(task_run_status_from_i16(0), D::Success);
        assert_eq!(task_run_status_from_i16(1), D::Failed);
        assert_eq!(task_run_status_from_i16(2), D::Running);
    }

    #[test]
    fn scheduler_task_store_to_domain_roundtrip() {
        let now = chrono::Utc::now();
        let id = Uuid::new_v4();
        let store_task = db_models::SchedulerTask {
            id,
            name: "job-discovery".into(),
            cron_expr: "0 */30 * * * *".into(),
            enabled: true,
            last_run_at: Some(now),
            last_status: Some(0),
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
        let now = chrono::Utc::now();
        let id = Uuid::new_v4();
        let task_id = Uuid::new_v4();
        let store_run = db_models::TaskRunHistory {
            id,
            task_id,
            status: 1,
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
