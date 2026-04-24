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
    schedule::{JobEntry, JobId, JobResult, Trigger},
    task_report::TaskReportStatus,
};
use serde::Serialize;
use uuid::Uuid;

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
    /// - `"awaiting_approval"` — latest run stopped waiting on user approval
    ///   (kernel [`TaskReportStatus::NeedsApproval`])
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
    /// The status mapping is authoritative — see [`status_label`] for the
    /// `TaskReportStatus` → wire-label contract used here.
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

/// Wire shape of `POST /api/v1/scheduler/jobs/{id}/trigger`.
///
/// Wraps the refreshed [`JobView`] with a `triggered` discriminator so the
/// frontend can distinguish a fresh dispatch (`true`) from a dedupe no-op
/// (`false`, the job was already running from a prior trigger) without
/// having to read an HTTP status code. Both outcomes are HTTP 200 — the
/// only error path is a genuine `JobNotFound` 404. This avoids asking the
/// web client to special-case a 409-style error for what is really an
/// idempotent operation.
#[derive(Debug, Serialize)]
pub struct TriggerJobView {
    /// Latest view of the job after the trigger attempt. `next_at` is
    /// unchanged by design — manual triggers never advance the schedule.
    #[serde(flatten)]
    pub view:      JobView,
    /// `true` when the syscall dispatched a fresh `ScheduledTask` event;
    /// `false` when an earlier trigger is still in-flight and the call was
    /// deduplicated.
    pub triggered: bool,
}

/// Wire shape of a single historical execution of a scheduled job.
///
/// Mirrors [`JobResult`] but normalises `status` through [`status_label`]
/// so `GET /jobs/:id/history` speaks the same vocabulary as
/// `GET /jobs[/:id]`'s `last_status`. The frontend no longer needs a
/// label shim to reconcile the two routes.
///
/// The internal `result` blob and routing `tags` are omitted — the admin
/// UI has no use for them today and they're load-bearing elsewhere in
/// the kernel.
#[derive(Debug, Serialize)]
pub struct JobResultView {
    /// The job that produced this result.
    pub job_id:       JobId,
    /// Task ID from the agent's TaskReport.
    pub task_id:      Uuid,
    /// Task type (e.g. `"pr_review"`).
    pub task_type:    String,
    /// Normalised status — see [`status_label`].
    pub status:       &'static str,
    /// Human-readable summary.
    pub summary:      String,
    /// Action taken by the agent, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_taken: Option<String>,
    /// When this execution completed.
    pub completed_at: Timestamp,
}

impl JobResultView {
    /// Build a [`JobResultView`] from a kernel [`JobResult`].
    pub fn from_result(result: JobResult) -> Self {
        Self {
            job_id:       result.job_id,
            task_id:      result.task_id,
            task_type:    result.task_type,
            status:       status_label(result.status),
            summary:      result.summary,
            action_taken: result.action_taken,
            completed_at: result.completed_at,
        }
    }
}

/// Map a kernel [`TaskReportStatus`] to the wire-level status label.
///
/// `NeedsApproval` surfaces as `"awaiting_approval"` — the agent has
/// stopped and is blocked on a human decision, which is a distinct
/// state from an in-progress run. The admin UI styles it as a warning
/// so users know action is required. Keep this mapping stable —
/// frontend code pattern-matches on the literal strings.
pub fn status_label(status: TaskReportStatus) -> &'static str {
    match status {
        TaskReportStatus::Completed => "ok",
        TaskReportStatus::Failed => "failed",
        TaskReportStatus::NeedsApproval => "awaiting_approval",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_label_covers_every_variant() {
        assert_eq!(status_label(TaskReportStatus::Completed), "ok");
        assert_eq!(status_label(TaskReportStatus::Failed), "failed");
        assert_eq!(
            status_label(TaskReportStatus::NeedsApproval),
            "awaiting_approval"
        );
    }
}
