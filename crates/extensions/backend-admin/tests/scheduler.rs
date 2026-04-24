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

//! Integration tests for the scheduler admin service.
//!
//! These exercise `SchedulerSvc` against a live kernel built by
//! [`TestKernelBuilder`]. The tests focus on the wiring that cannot be
//! covered by unit tests in `dto.rs` / `router.rs`:
//!
//! - Service round-trip against a real `KernelHandle` (not a mock).
//! - 404 translation: missing jobs surface as `SchedulerError::JobNotFound`
//!   through the syscall boundary.
//! - History reads against the real OpenDAL-backed `JobResultStore`, including
//!   the newest-first + limit-trimming contract.

use jiff::Timestamp;
use rara_backend_admin::scheduler::service::{SchedulerError, SchedulerSvc};
use rara_kernel::{
    schedule::{JobId, JobResult},
    task_report::TaskReportStatus,
    testing::TestKernelBuilder,
};

/// Fresh kernel: no jobs registered, the admin list is empty and lookups
/// on a fabricated ID surface as `JobNotFound`.
#[tokio::test]
async fn list_on_empty_kernel_is_empty() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let tk = TestKernelBuilder::new(tmp.path()).build().await;
    let svc = SchedulerSvc::new(tk.handle.clone());

    let jobs = svc.list_jobs().await;
    assert!(jobs.is_empty(), "expected empty kernel, got {jobs:?}");

    tk.shutdown();
}

#[tokio::test]
async fn get_missing_returns_job_not_found() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let tk = TestKernelBuilder::new(tmp.path()).build().await;
    let svc = SchedulerSvc::new(tk.handle.clone());

    let ghost = JobId::new();
    let err = svc
        .get_job(&ghost)
        .await
        .expect_err("ghost job must not be found");
    assert!(
        matches!(err, SchedulerError::JobNotFound { .. }),
        "expected JobNotFound, got {err:?}"
    );

    tk.shutdown();
}

#[tokio::test]
async fn delete_missing_returns_job_not_found() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let tk = TestKernelBuilder::new(tmp.path()).build().await;
    let svc = SchedulerSvc::new(tk.handle.clone());

    let ghost = JobId::new();
    let err = svc
        .delete_job(&ghost)
        .await
        .expect_err("delete on ghost id must fail");
    assert!(
        matches!(err, SchedulerError::JobNotFound { .. }),
        "expected JobNotFound, got {err:?}"
    );

    tk.shutdown();
}

#[tokio::test]
async fn trigger_missing_returns_job_not_found() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let tk = TestKernelBuilder::new(tmp.path()).build().await;
    let svc = SchedulerSvc::new(tk.handle.clone());

    let ghost = JobId::new();
    let err = svc
        .trigger_job(&ghost)
        .await
        .expect_err("trigger on ghost id must fail");
    assert!(
        matches!(err, SchedulerError::JobNotFound { .. }),
        "expected JobNotFound, got {err:?}"
    );

    tk.shutdown();
}

/// Exercise the OpenDAL-backed `JobResultStore` through the service's
/// history method: insert three results out-of-order, then assert the
/// service returns them newest-first and honours `limit`.
///
/// The store is reached via `KernelHandle::job_result_store()` — this
/// verifies both the new accessor wiring and the service's own
/// reverse+truncate contract.
#[tokio::test]
async fn history_returns_newest_first_and_respects_limit() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let tk = TestKernelBuilder::new(tmp.path()).build().await;
    let svc = SchedulerSvc::new(tk.handle.clone());

    let job_id = JobId::new();
    let store = tk.handle.job_result_store().clone();

    // Seed three results with increasing completion timestamps.
    let base = Timestamp::from_second(1_800_000_000).expect("fixed ts");
    for offset in [0, 60, 120] {
        let completed_at = Timestamp::from_second(base.as_second() + offset).unwrap();
        store
            .append(&JobResult {
                job_id,
                task_id: uuid::Uuid::new_v4(),
                task_type: "test".into(),
                tags: vec![],
                status: TaskReportStatus::Completed,
                summary: format!("run at {completed_at}"),
                result: serde_json::json!({"at": completed_at.as_second()}),
                action_taken: None,
                completed_at,
            })
            .await
            .expect("append result");
    }

    // limit=2 should return the two newest (offset 120 then 60).
    let page = svc.history(&job_id, 2).await;
    assert_eq!(page.len(), 2, "limit should cap at 2");
    assert_eq!(
        page[0].completed_at.as_second(),
        base.as_second() + 120,
        "newest result should come first"
    );
    assert_eq!(
        page[1].completed_at.as_second(),
        base.as_second() + 60,
        "second-newest should follow"
    );

    // limit larger than result count returns all three, still newest-first.
    let page = svc.history(&job_id, 50).await;
    assert_eq!(page.len(), 3);
    assert_eq!(page[0].completed_at.as_second(), base.as_second() + 120);
    assert_eq!(page[2].completed_at.as_second(), base.as_second());

    tk.shutdown();
}
