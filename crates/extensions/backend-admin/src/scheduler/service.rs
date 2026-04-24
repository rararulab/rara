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

//! Service layer for the scheduler admin API.
//!
//! Read paths (`list_jobs`, `get_job`, `history`) go straight to the
//! kernel's in-memory `JobWheel` and the on-disk `JobResultStore` — no
//! syscall round-trip is required because these operations are pure
//! lookups.
//!
//! Mutations (`delete_job`, `trigger_job`) flow through the kernel event
//! queue so the single-writer invariant on the wheel is preserved and the
//! `TriggerJob` path emits the same `ScheduledTask` event a natural
//! drain would produce.

use rara_kernel::{
    event::{KernelEventEnvelope, Syscall},
    handle::KernelHandle,
    schedule::{JobEntry, JobId},
    session::SessionKey,
};
use snafu::{ResultExt, Snafu};
use tokio::sync::oneshot;
use tracing::instrument;

use super::dto::{JobResultView, JobView};

/// Service-level errors surfaced to the HTTP handlers.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum SchedulerError {
    /// No job with the supplied ID exists on the wheel.
    #[snafu(display("job not found: {job_id}"))]
    JobNotFound { job_id: String },
    /// Kernel dropped the syscall reply channel — event queue or kernel
    /// shut down mid-call.
    #[snafu(display("kernel dropped reply channel: {source}"))]
    ReplyDropped { source: oneshot::error::RecvError },
    /// Event queue is full or closed; the syscall was not delivered.
    #[snafu(display("event queue unavailable: {message}"))]
    EventQueue { message: String },
    /// Kernel returned an error executing the syscall.
    #[snafu(display("kernel syscall failed: {source}"))]
    Kernel {
        source: rara_kernel::error::KernelError,
    },
}

/// Per-crate result alias.
pub type Result<T> = std::result::Result<T, SchedulerError>;

/// Scheduler admin service.
///
/// Owns nothing — it's a thin adapter over a cloned [`KernelHandle`]. Cheap
/// to clone (two `Arc` bumps) so each HTTP handler can take it by value.
#[derive(Clone)]
pub struct SchedulerSvc {
    handle: KernelHandle,
}

impl SchedulerSvc {
    /// Construct a service bound to the given kernel handle.
    pub fn new(handle: KernelHandle) -> Self { Self { handle } }

    /// List every scheduled job across all sessions.
    ///
    /// Derives `last_status` / `last_run_at` per job by calling
    /// [`rara_kernel::schedule::JobResultStore::read_latest`]. Each lookup
    /// is an OpenDAL `list` + single `read`, so this scales linearly with
    /// the number of jobs — acceptable for an admin curation surface.
    #[instrument(skip_all)]
    pub async fn list_jobs(&self) -> Vec<JobView> {
        let jobs = self.handle.list_jobs(None);
        let store = self.handle.job_result_store();
        let mut out = Vec::with_capacity(jobs.len());
        for job in jobs {
            let latest = store.read_latest(&job.id).await;
            out.push(JobView::from_job(job, latest.as_ref()));
        }
        out
    }

    /// Fetch a single job view by ID, or return
    /// [`SchedulerError::JobNotFound`].
    #[instrument(skip_all, fields(%job_id))]
    pub async fn get_job(&self, job_id: &JobId) -> Result<JobView> {
        let job = self.find_job(job_id)?;
        let latest = self.handle.job_result_store().read_latest(job_id).await;
        Ok(JobView::from_job(job, latest.as_ref()))
    }

    /// Remove a job from the wheel via the `RemoveJob` syscall.
    ///
    /// Returns [`SchedulerError::JobNotFound`] if the kernel reports the
    /// job was not on the wheel.
    #[instrument(skip_all, fields(%job_id))]
    pub async fn delete_job(&self, job_id: &JobId) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.push_syscall(Syscall::RemoveJob {
            job_id:   *job_id,
            reply_tx: tx,
        })?;
        match rx.await.context(ReplyDroppedSnafu)? {
            Ok(()) => Ok(()),
            // The kernel returns a generic `KernelError::Other` for missing
            // jobs; translate to the typed 404 variant here so the HTTP
            // layer can distinguish a legitimate miss from an infra fault.
            Err(_) => Err(SchedulerError::JobNotFound {
                job_id: job_id.to_string(),
            }),
        }
    }

    /// Fire a job on demand without advancing its `next_at`.
    ///
    /// Returns the refreshed [`JobView`] so the HTTP layer can respond with
    /// the post-trigger state. The `next_at` in the view is unchanged from
    /// before the call — that invariant is enforced inside the kernel by
    /// [`rara_kernel::schedule::JobWheel::trigger_now`].
    #[instrument(skip_all, fields(%job_id))]
    pub async fn trigger_job(&self, job_id: &JobId) -> Result<JobView> {
        let (tx, rx) = oneshot::channel();
        self.push_syscall(Syscall::TriggerJob {
            job_id:   *job_id,
            reply_tx: tx,
        })?;
        match rx.await.context(ReplyDroppedSnafu)? {
            Ok(()) => self.get_job(job_id).await,
            Err(_) => Err(SchedulerError::JobNotFound {
                job_id: job_id.to_string(),
            }),
        }
    }

    /// Read up to `limit` most recent execution results for `job_id`,
    /// newest first.
    ///
    /// Results are normalised through [`JobResultView`] so their `status`
    /// field matches the `last_status` vocabulary used by [`JobView`] —
    /// the frontend pattern-matches on one label space across both routes.
    #[instrument(skip_all, fields(%job_id, limit))]
    pub async fn history(&self, job_id: &JobId, limit: usize) -> Vec<JobResultView> {
        let mut results = self.handle.job_result_store().read(job_id).await;
        // `JobResultStore::read` yields ascending-by-time; admin callers
        // want newest-first paging so they don't have to flip the slice
        // themselves.
        results.reverse();
        results.truncate(limit);
        results
            .into_iter()
            .map(JobResultView::from_result)
            .collect()
    }

    // -- internals ----------------------------------------------------------

    /// Look up a job on the wheel by ID.
    fn find_job(&self, job_id: &JobId) -> Result<JobEntry> {
        self.handle
            .list_jobs(None)
            .into_iter()
            .find(|j| j.id == *job_id)
            .ok_or_else(|| SchedulerError::JobNotFound {
                job_id: job_id.to_string(),
            })
    }

    /// Push a session-scoped syscall on behalf of the admin route.
    ///
    /// The admin surface has no real session, but the scheduler syscalls
    /// we use here (`RemoveJob`, `TriggerJob`) operate directly on the
    /// wheel and ignore `syscall_sender` — a fresh `SessionKey` is
    /// sufficient to route the envelope through the queue.
    fn push_syscall(&self, syscall: Syscall) -> Result<()> {
        let envelope = KernelEventEnvelope::session_command(SessionKey::new(), syscall);
        self.handle
            .event_queue()
            .try_push(envelope)
            .map_err(|_| SchedulerError::EventQueue {
                message: "event queue full or closed".into(),
            })
    }
}
