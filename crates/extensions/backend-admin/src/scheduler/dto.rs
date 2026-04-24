// Copyright 2025 Rararulab
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

//! Wire-level DTOs for the scheduler admin API.
//!
//! These types shape what goes over HTTP. They deliberately omit internal
//! kernel state — notably `Principal`, which is security-sensitive and
//! never appears in admin responses.

use jiff::Timestamp;
use rara_kernel::{
    schedule::{JobEntry, JobResult, Trigger},
    task_report::TaskReportStatus,
};
use serde::Serialize;

/// Public wire shape of a scheduled job.
///
/// `Principal` is intentionally stripped — it's kernel-internal and must
/// not leak to admin clients. `Trigger` passes through with the kernel's
/// existing serde tags (`once` / `interval` / `cron`).
#[derive(Debug, Serialize)]
pub struct JobView {
    /// Job identifier (stringified UUID).
    pub id:          String,
    /// When / how this job fires — kernel `Trigger` enum verbatim.
    pub trigger:     Trigger,
    /// Text injected as a `UserMessage` when the job fires.
    pub message:     String,
    /// Session the job is bound to.
    pub session_key: String,
    /// Routing tags propagated to `TaskNotification` on completion.
    pub tags:        Vec<String>,
    /// When this job was created.
    pub created_at:  Timestamp,
    /// Condensed status of the most recent execution.
    ///
    /// - `"ok"` — latest run completed successfully
    /// - `"failed"` — latest run failed
    /// - `"running"` — latest run awaits user approval (kernel
    ///   [`TaskReportStatus::NeedsApproval`])
    /// - `null` — the job has never executed (no results on disk)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_status: Option<&'static str>,
    /// `completed_at` of the most recent execution, or `null` if absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run_at: Option<Timestamp>,
}

impl JobView {
    /// Build a [`JobView`] from a kernel [`JobEntry`] and (optionally) the
    /// most recent [`JobResult`] for that job.
    ///
    /// The status mapping is authoritative — see this module's doc comment
    /// for the rationale behind `NeedsApproval` → `"running"`.
    pub fn from_job(job: JobEntry, latest: Option<&JobResult>) -> Self {
        let last_status = latest.map(|r| status_label(r.status));
        let last_run_at = latest.map(|r| r.completed_at);
        Self {
            id: job.id.to_string(),
            trigger: job.trigger,
            message: job.message,
            session_key: job.session_key.to_string(),
            tags: job.tags,
            created_at: job.created_at,
            last_status,
            last_run_at,
        }
    }
}

/// Map a kernel [`TaskReportStatus`] to the wire-level status label.
///
/// `NeedsApproval` folds into `"running"` so the admin UI shows jobs
/// blocked on user approval as still in-flight rather than finished.
/// Keep this mapping stable — frontend code pattern-matches on the
/// literal strings.
pub fn status_label(status: TaskReportStatus) -> &'static str {
    match status {
        TaskReportStatus::Completed => "ok",
        TaskReportStatus::Failed => "failed",
        TaskReportStatus::NeedsApproval => "running",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_label_covers_every_variant() {
        assert_eq!(status_label(TaskReportStatus::Completed), "ok");
        assert_eq!(status_label(TaskReportStatus::Failed), "failed");
        assert_eq!(status_label(TaskReportStatus::NeedsApproval), "running");
    }
}
