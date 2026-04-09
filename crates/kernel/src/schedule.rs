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

//! Scheduled task system — timing-wheel driven job scheduling.
//!
//! Provides [`JobEntry`] (a scheduled task), [`Trigger`] (when to fire),
//! and [`JobWheel`] (the scheduling data structure backed by a `BTreeMap`).
//! Jobs are persisted as JSON and restored on startup.

use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
};

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use uuid::Uuid;

use crate::{identity::Principal, session::SessionKey};

base::define_id!(
    /// Unique identifier for a scheduled job.
    JobId
);

// ---------------------------------------------------------------------------
// Trigger
// ---------------------------------------------------------------------------

/// When a job should fire.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Trigger {
    /// Fire once at a specific time, then remove.
    Once { run_at: Timestamp },
    /// Fire at fixed intervals.
    Interval {
        /// Interval in seconds.
        every_secs: u64,
        /// Next scheduled fire time.
        next_at:    Timestamp,
    },
    /// Fire according to a cron expression.
    Cron {
        /// The cron expression string (e.g. `"0 9 * * *"`).
        expr:    String,
        /// Next scheduled fire time.
        next_at: Timestamp,
    },
}

impl Trigger {
    /// The next time this trigger should fire.
    pub fn next_at(&self) -> Timestamp {
        match self {
            Trigger::Once { run_at } => *run_at,
            Trigger::Interval { next_at, .. } => *next_at,
            Trigger::Cron { next_at, .. } => *next_at,
        }
    }

    /// Human-readable summary of the trigger schedule.
    pub fn summary(&self) -> String {
        match self {
            Trigger::Once { run_at } => format!("once at {run_at}"),
            Trigger::Interval { every_secs, .. } => {
                if *every_secs >= 3600 && every_secs % 3600 == 0 {
                    format!("every {}h", every_secs / 3600)
                } else if *every_secs >= 60 && every_secs % 60 == 0 {
                    format!("every {}m", every_secs / 60)
                } else {
                    format!("every {every_secs}s")
                }
            }
            Trigger::Cron { expr, .. } => format!("cron {expr}"),
        }
    }
}

// ---------------------------------------------------------------------------
// JobEntry
// ---------------------------------------------------------------------------

/// A single scheduled job entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobEntry {
    /// Unique job identifier.
    pub id:          JobId,
    /// When/how this job fires.
    pub trigger:     Trigger,
    /// Text injected as a `UserMessage` when the job fires.
    pub message:     String,
    /// Session this job is bound to.
    pub session_key: SessionKey,
    /// The principal who created the job.
    pub principal:   Principal,
    /// When this job was created.
    pub created_at:  Timestamp,
    /// Routing tags propagated to TaskNotification on completion.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags:        Vec<String>,
}

// ---------------------------------------------------------------------------
// DrainResult — output of JobWheel::drain_expired
// ---------------------------------------------------------------------------

/// Result of draining expired jobs from the wheel.
///
/// Splits drained jobs into two buckets so the caller can fire normal jobs
/// and notify the user about cron jobs whose expression no longer yields a
/// future time. Keeping this split out of the wheel preserves the sans-IO
/// invariant — the wheel never touches the notification bus directly.
#[derive(Debug, Default)]
pub struct DrainResult {
    /// Jobs whose `next_at` has passed and that should be dispatched now.
    pub fired:        Vec<JobEntry>,
    /// Cron jobs that were removed because their expression yields no
    /// future fire time (e.g., the last valid date has passed). The caller
    /// is responsible for emitting a user-visible notification.
    pub cron_expired: Vec<JobEntry>,
}

// ---------------------------------------------------------------------------
// JobWheel — BTreeMap-backed scheduling structure
// ---------------------------------------------------------------------------

/// BTreeMap key for the job wheel: `(unix_seconds, uuid)`.
///
/// Using raw primitives that implement `Ord` rather than newtype wrappers
/// (which only derive `Eq`).
type WheelKey = (i64, Uuid);

/// A simple scheduling structure backed by a `BTreeMap<WheelKey, JobEntry>`.
///
/// Jobs are keyed by `(next_at_seconds, job_uuid)` so `drain_expired` can
/// efficiently pop all entries whose time has passed.
///
/// An **in-flight ledger** tracks jobs that have been drained but whose
/// execution agent has not yet completed. On startup, any in-flight jobs
/// are re-fired so that a kernel crash between drain and `publish_report`
/// does not silently lose task results.
pub struct JobWheel {
    /// Jobs ordered by (next_fire_time_secs, job_uuid).
    jobs:                BTreeMap<WheelKey, JobEntry>,
    /// Jobs that have been drained and dispatched but not yet completed.
    in_flight:           HashMap<JobId, JobEntry>,
    /// Path to the `jobs.json` persistence file.
    path:                PathBuf,
    /// Runtime-only flag: true once `take_in_flight` has returned recovered
    /// jobs. Prevents re-firing on subsequent ticks without clearing the
    /// ledger prematurely — entries are removed individually by
    /// `complete_in_flight` after the agent session ends.
    in_flight_recovered: bool,
}

impl JobWheel {
    /// Build a wheel key from a job entry.
    fn key(entry: &JobEntry) -> WheelKey {
        (entry.trigger.next_at().as_second(), entry.id.as_uuid())
    }

    /// Derive the in-flight ledger path from the jobs.json path.
    fn in_flight_path(jobs_path: &std::path::Path) -> PathBuf {
        jobs_path.with_file_name("in_flight.json")
    }

    /// Load jobs and in-flight ledger from disk, or create an empty wheel.
    pub fn load(path: PathBuf) -> Self {
        let jobs = match std::fs::read_to_string(&path) {
            Ok(content) => {
                let entries: Vec<JobEntry> = match serde_json::from_str(&content) {
                    Ok(v) => v,
                    Err(e) => {
                        warn!(error = %e, path = %path.display(), "failed to parse jobs.json, starting empty");
                        Vec::new()
                    }
                };
                let mut map = BTreeMap::new();
                for entry in entries {
                    let key = Self::key(&entry);
                    map.insert(key, entry);
                }
                info!(count = map.len(), "restored scheduled jobs from disk");
                map
            }
            Err(_) => {
                info!(path = %path.display(), "no jobs.json found, starting empty");
                BTreeMap::new()
            }
        };

        let ifl_path = Self::in_flight_path(&path);
        let in_flight = match std::fs::read_to_string(&ifl_path) {
            Ok(content) => {
                let entries: Vec<JobEntry> = match serde_json::from_str(&content) {
                    Ok(v) => v,
                    Err(e) => {
                        warn!(error = %e, path = %ifl_path.display(), "failed to parse in_flight.json, starting empty");
                        Vec::new()
                    }
                };
                let map: HashMap<JobId, JobEntry> =
                    entries.into_iter().map(|e| (e.id, e)).collect();
                if !map.is_empty() {
                    info!(count = map.len(), "restored in-flight jobs from disk");
                }
                map
            }
            Err(_) => HashMap::new(),
        };

        Self {
            jobs,
            in_flight,
            path,
            in_flight_recovered: false,
        }
    }

    /// Return the next fire time, or `None` if the wheel is empty.
    pub fn next_deadline(&self) -> Option<jiff::Timestamp> {
        self.jobs
            .keys()
            .next()
            .map(|(secs, _)| jiff::Timestamp::from_second(*secs).unwrap())
    }

    /// Drain all jobs whose `next_at` is at or before `now`.
    ///
    /// - `Once` jobs are removed from the wheel.
    /// - `Interval` jobs have their `next_at` advanced and are re-inserted.
    /// - `Cron` jobs compute the next fire time from their expression and are
    ///   re-inserted. If the cron expression yields no future time, the job is
    ///   moved to [`DrainResult::cron_expired`] so the caller can notify the
    ///   user — the wheel itself stays sans-IO.
    ///
    /// All fired jobs are placed in the **in-flight ledger** so they can
    /// be re-fired on startup if the kernel crashes before the execution
    /// agent completes. Cron-expired jobs are NOT placed in the in-flight
    /// ledger because they will not be re-executed.
    pub fn drain_expired(&mut self, now: Timestamp) -> DrainResult {
        let mut fired = Vec::new();
        let mut cron_expired = Vec::new();
        let cutoff: WheelKey = (now.as_second(), Uuid::max());

        // Collect all keys up to `now`.
        let keys: Vec<WheelKey> = self.jobs.range(..=cutoff).map(|(k, _)| *k).collect();

        for key in keys {
            let Some(entry) = self.jobs.remove(&key) else {
                continue;
            };

            // For cron jobs, peek at the next fire time before committing —
            // an unsatisfiable expression means the job is dead and must be
            // surfaced to the user instead of silently fired-then-deleted.
            if let Trigger::Cron { expr, .. } = &entry.trigger {
                if Self::next_cron_time(expr, now).is_none() {
                    warn!(
                        job_id = %entry.id,
                        expr = %expr,
                        "cron expression yields no future time, removing job"
                    );
                    cron_expired.push(entry);
                    continue;
                }
            }

            // Record in the in-flight ledger before dispatching.
            self.in_flight.insert(entry.id, entry.clone());
            fired.push(entry.clone());

            // Re-schedule recurring jobs.
            match entry.trigger.clone() {
                Trigger::Once { .. } => {
                    // One-shot — do not re-insert into the wheel.
                }
                Trigger::Interval { every_secs, .. } => {
                    let next = now
                        .checked_add(jiff::SignedDuration::from_secs(every_secs as i64))
                        .unwrap_or(now);

                    let mut rescheduled = entry;
                    rescheduled.trigger = Trigger::Interval {
                        every_secs,
                        next_at: next,
                    };
                    self.jobs.insert(Self::key(&rescheduled), rescheduled);
                }
                Trigger::Cron { expr, .. } => {
                    // We already verified `next_cron_time` is `Some` above,
                    // but recompute defensively rather than threading state.
                    if let Some(next) = Self::next_cron_time(&expr, now) {
                        let mut rescheduled = entry;
                        rescheduled.trigger = Trigger::Cron {
                            expr,
                            next_at: next,
                        };
                        self.jobs.insert(Self::key(&rescheduled), rescheduled);
                    }
                }
            }
        }

        DrainResult {
            fired,
            cron_expired,
        }
    }

    /// Add a job to the wheel.
    pub fn add(&mut self, entry: JobEntry) {
        let key = Self::key(&entry);
        self.jobs.insert(key, entry);
    }

    /// Remove a job by ID. Returns the removed entry if found.
    pub fn remove(&mut self, id: &JobId) -> Option<JobEntry> {
        let key = self.jobs.iter().find(|(_, v)| v.id == *id).map(|(k, _)| *k);
        key.and_then(|k| self.jobs.remove(&k))
    }

    /// List all jobs, optionally filtered by session key.
    pub fn list(&self, session_key: Option<&SessionKey>) -> Vec<JobEntry> {
        self.jobs
            .values()
            .filter(|e| session_key.map_or(true, |sk| e.session_key == *sk))
            .cloned()
            .collect()
    }

    /// Mark a job as completed, removing it from the in-flight ledger.
    ///
    /// Called when the execution agent's session ends (regardless of whether
    /// `publish_report` was called). Persists the updated ledger to disk.
    pub fn complete_in_flight(&mut self, job_id: &JobId) -> bool {
        let removed = self.in_flight.remove(job_id).is_some();
        if removed {
            self.persist_in_flight();
        }
        removed
    }

    /// Return in-flight jobs from a previous run for re-firing on startup.
    ///
    /// Returns clones on the first call and sets a flag so subsequent calls
    /// return empty. The ledger is **not** cleared here — entries are removed
    /// individually by [`JobWheel::complete_in_flight`] after each agent
    /// session ends. This makes the recovery crash-safe: if the kernel
    /// crashes again before the re-fired agents finish, the ledger still
    /// contains the entries and they will be recovered on the next startup.
    pub fn take_in_flight(&mut self) -> Vec<JobEntry> {
        if self.in_flight_recovered || self.in_flight.is_empty() {
            return Vec::new();
        }
        self.in_flight_recovered = true;
        let jobs: Vec<JobEntry> = self.in_flight.values().cloned().collect();
        info!(
            count = jobs.len(),
            "re-firing in-flight jobs from previous run"
        );
        jobs
    }

    /// Persist the current wheel state to the JSON file.
    pub fn persist(&self) {
        self.persist_jobs();
        self.persist_in_flight();
    }

    /// Persist only the jobs BTreeMap.
    fn persist_jobs(&self) {
        let entries: Vec<&JobEntry> = self.jobs.values().collect();
        match serde_json::to_string_pretty(&entries) {
            Ok(json) => {
                if let Some(parent) = self.path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if let Err(e) = std::fs::write(&self.path, json) {
                    warn!(error = %e, path = %self.path.display(), "failed to persist jobs.json");
                }
            }
            Err(e) => {
                warn!(error = %e, "failed to serialize jobs for persistence");
            }
        }
    }

    /// Persist only the in-flight ledger.
    fn persist_in_flight(&self) {
        let ifl_path = Self::in_flight_path(&self.path);
        let entries: Vec<&JobEntry> = self.in_flight.values().collect();
        match serde_json::to_string_pretty(&entries) {
            Ok(json) => {
                if let Some(parent) = ifl_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if let Err(e) = std::fs::write(&ifl_path, json) {
                    warn!(error = %e, path = %ifl_path.display(), "failed to persist in_flight.json");
                }
            }
            Err(e) => {
                warn!(error = %e, "failed to serialize in-flight jobs for persistence");
            }
        }
    }

    /// Compute the next fire time for a cron expression after `after`.
    ///
    /// Returns `None` if the expression is unparseable or if it has no
    /// upcoming time after `after` (e.g., `0 0 0 31 2 *` — Feb 31 never
    /// happens). Used both at registration time (by the schedule tool) to
    /// reject impossible expressions and at drain time to detect cron jobs
    /// whose last valid date has passed.
    pub(crate) fn next_cron_time(expr: &str, after: Timestamp) -> Option<Timestamp> {
        use std::str::FromStr;

        let schedule = cron::Schedule::from_str(expr).ok()?;
        // Convert jiff::Timestamp to chrono::DateTime<Utc>.
        let after_chrono = chrono::DateTime::<chrono::Utc>::from_timestamp(
            after.as_second(),
            after.subsec_nanosecond() as u32,
        )?;
        let next_chrono = schedule.upcoming(chrono::Utc).find(|t| *t > after_chrono)?;
        let next_ts = Timestamp::from_second(next_chrono.timestamp()).ok()?;
        Some(next_ts)
    }
}

// ---------------------------------------------------------------------------
// JobResult & JobResultStore — per-job append-only result log
// ---------------------------------------------------------------------------

/// A single execution result for a scheduled job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobResult {
    /// The job that produced this result.
    pub job_id:       JobId,
    /// Task ID from the agent's TaskReport.
    pub task_id:      Uuid,
    /// Task type (e.g. "pr_review").
    pub task_type:    String,
    /// Routing tags.
    pub tags:         Vec<String>,
    /// Completion status.
    pub status:       crate::task_report::TaskReportStatus,
    /// Human-readable summary.
    pub summary:      String,
    /// Structured result data.
    pub result:       serde_json::Value,
    /// Action taken by the agent, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_taken: Option<String>,
    /// When this execution completed.
    pub completed_at: Timestamp,
}

/// Append-only store for job execution results backed by OpenDAL.
///
/// Storage layout: `{job_id}/{completed_at_epoch}.json` — one object per
/// execution. For once-jobs there is exactly one object; for recurring
/// jobs each execution adds a new object.
///
/// Uses the OpenDAL `Fs` service so results survive kernel restarts and
/// the backend can be swapped to S3/GCS later without code changes.
pub struct JobResultStore {
    op: opendal::Operator,
}

impl JobResultStore {
    /// Create a new result store rooted at `results_dir`.
    pub fn new(results_dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&results_dir);
        let op = opendal::Operator::new(
            opendal::services::Fs::default().root(&results_dir.to_string_lossy()),
        )
        .expect("Fs operator should be infallible")
        .finish();
        Self { op }
    }

    /// Write an execution result as a new object.
    ///
    /// Object key: `{job_id}/{completed_at_epoch}.json`
    pub async fn append(&self, result: &JobResult) -> anyhow::Result<()> {
        let key = format!("{}/{}.json", result.job_id, result.completed_at.as_second());
        let bytes = serde_json::to_vec_pretty(result)?;
        self.op.write(&key, bytes).await?;
        Ok(())
    }

    /// Read all execution results for a given job, ordered by completion
    /// time (lexicographic on the epoch filename).
    pub async fn read(&self, job_id: &JobId) -> Vec<JobResult> {
        let prefix = format!("{job_id}/");
        let mut entries = match self.op.list(&prefix).await {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };
        // Sort by path (epoch filenames sort chronologically).
        entries.sort_by(|a, b| a.path().cmp(b.path()));

        let mut results = Vec::new();
        for entry in entries {
            if entry.metadata().is_dir() {
                continue;
            }
            match self.op.read(entry.path()).await {
                Ok(buf) => match serde_json::from_slice::<JobResult>(&buf.to_vec()) {
                    Ok(r) => results.push(r),
                    Err(e) => {
                        warn!(error = %e, path = entry.path(), "skipping malformed job result");
                    }
                },
                Err(e) => {
                    warn!(error = %e, path = entry.path(), "failed to read job result");
                }
            }
        }
        results
    }
}
