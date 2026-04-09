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
    sync::{Arc, Mutex},
};

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use uuid::Uuid;

use crate::{identity::Principal, session::SessionKey};

// ---------------------------------------------------------------------------
// Clock — abstraction over wall-clock time for deterministic tests
// ---------------------------------------------------------------------------

/// Abstract clock for testability.
///
/// Production code uses [`SystemClock`], which delegates to
/// `jiff::Timestamp::now()`. Tests use [`FakeClock`], which can be advanced
/// manually so scheduler behaviour does not depend on real wall-clock time.
pub trait Clock: Send + Sync + 'static {
    /// The current wall-clock instant according to this clock.
    fn now(&self) -> Timestamp;
}

/// Type alias for a shared clock handle.
pub type ClockRef = Arc<dyn Clock>;

/// Real system clock wrapping `jiff::Timestamp::now()`.
#[derive(Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp { Timestamp::now() }
}

/// Test clock whose current time can be set or advanced manually.
///
/// Used to make scheduler tests deterministic — no `tokio::time::sleep`
/// or wall-clock dependencies.
#[derive(Debug)]
pub struct FakeClock {
    inner: Mutex<Timestamp>,
}

impl FakeClock {
    /// Create a new `FakeClock` initialised to `start`.
    pub fn new(start: Timestamp) -> Self {
        Self {
            inner: Mutex::new(start),
        }
    }

    /// Advance the clock by `delta`.
    ///
    /// Panics if `delta` cannot be converted to a `SignedDuration` or if
    /// the resulting timestamp overflows — both indicate a buggy test.
    pub fn advance(&self, delta: std::time::Duration) {
        let signed = jiff::SignedDuration::try_from(delta)
            .expect("FakeClock::advance: duration must fit in SignedDuration");
        let mut guard = self.inner.lock().expect("FakeClock mutex poisoned");
        *guard = guard
            .checked_add(signed)
            .expect("FakeClock::advance: timestamp overflow");
    }

    /// Set the clock to an absolute timestamp.
    pub fn set(&self, ts: Timestamp) { *self.inner.lock().expect("FakeClock mutex poisoned") = ts; }
}

impl Clock for FakeClock {
    fn now(&self) -> Timestamp { *self.inner.lock().expect("FakeClock mutex poisoned") }
}

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
        /// Original reference point for interval scheduling. The next fire
        /// time is always computed as `anchor_at + k * every_secs` for the
        /// smallest `k` placing the result strictly after `now` — this gives
        /// drift-free, catch-up semantics across missed periods, sleep, or
        /// scheduler stalls.
        ///
        /// Optional in serde to remain backward compatible with legacy
        /// `jobs.json` files written before this field existed; missing
        /// entries are backfilled from `next_at` at load time.
        #[serde(default)]
        anchor_at:  Option<Timestamp>,
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
    /// The principal who created the job. Always fully resolved — jobs are
    /// registered through `Syscall::RegisterJob`, which copies the resolved
    /// principal off the originating session in the process table.
    pub principal:   Principal<crate::identity::Resolved>,
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
// InFlightEntry — in-flight ledger entry with lease deadline
// ---------------------------------------------------------------------------

/// Default lease duration in seconds (5 minutes). If a job's execution agent
/// does not call `complete_in_flight` within this window, the entry is
/// considered orphaned and will be discarded on the next restart instead of
/// re-fired forever.
const DEFAULT_LEASE_SECS: i64 = 300;

/// An in-flight job wrapped with lease metadata.
///
/// Persisted to `in_flight.json` so that on restart the kernel can distinguish
/// between jobs that are still plausibly running (lease not yet expired) and
/// orphaned entries whose agent crashed or hung.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InFlightEntry {
    /// The original job that was dispatched.
    pub job:            JobEntry,
    /// When the job was drained from the wheel and dispatched.
    pub fired_at:       Timestamp,
    /// Absolute deadline by which `complete_in_flight` must be called.
    /// Entries past this deadline are discarded on recovery rather than
    /// re-fired.
    pub lease_deadline: Timestamp,
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
    /// Sidecar index from `JobId` to its current `WheelKey`, enabling O(1)
    /// `remove`. Invariant: `by_id` contains exactly the same job ids as
    /// `jobs`, and each value is the key under which that job lives in
    /// `jobs`. All mutators of `jobs` must update `by_id` in lockstep.
    by_id:               HashMap<JobId, WheelKey>,
    /// Jobs that have been drained and dispatched but not yet completed.
    /// Each entry carries a lease deadline; orphaned entries past their
    /// deadline are discarded on recovery instead of re-fired.
    in_flight:           HashMap<JobId, InFlightEntry>,
    /// Path to the `jobs.json` persistence file.
    path:                PathBuf,
    /// Runtime-only flag: true once `take_in_flight` has returned recovered
    /// jobs. Prevents re-firing on subsequent ticks without clearing the
    /// ledger prematurely — entries are removed individually by
    /// `complete_in_flight` after the agent session ends.
    in_flight_recovered: bool,
    /// Clock used for "now" inside the wheel. Production wires
    /// [`SystemClock`]; tests inject a [`FakeClock`].
    clock:               ClockRef,
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
    ///
    /// Uses [`SystemClock`] as the wall-clock source. Tests should call
    /// [`JobWheel::load_with_clock`] with a [`FakeClock`] instead.
    pub fn load(path: PathBuf) -> Self { Self::load_with_clock(path, Arc::new(SystemClock)) }

    /// Like [`JobWheel::load`] but with an injected clock — used by tests
    /// to make scheduling deterministic.
    pub fn load_with_clock(path: PathBuf, clock: ClockRef) -> Self {
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
                for mut entry in entries {
                    // Backfill `anchor_at` for legacy interval entries
                    // serialized before the field existed. Using the prior
                    // `next_at` as the anchor preserves the observable
                    // cadence: subsequent fires land on
                    // `anchor + k * every_secs`, which lines up with the
                    // unmodified `next_at` value the old scheduler would
                    // have produced.
                    if let Trigger::Interval {
                        anchor_at: anchor_at @ None,
                        next_at,
                        ..
                    } = &mut entry.trigger
                    {
                        *anchor_at = Some(*next_at);
                    }
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

        // Rebuild the sidecar index from the loaded BTreeMap so the
        // O(1) `remove` invariant holds immediately after `load`.
        let by_id: HashMap<JobId, WheelKey> = jobs.iter().map(|(k, v)| (v.id, *k)).collect();

        let ifl_path = Self::in_flight_path(&path);
        let in_flight = match std::fs::read_to_string(&ifl_path) {
            Ok(content) => {
                let entries: Vec<InFlightEntry> = match serde_json::from_str(&content) {
                    Ok(v) => v,
                    Err(e) => {
                        warn!(error = %e, path = %ifl_path.display(), "failed to parse in_flight.json, starting empty");
                        Vec::new()
                    }
                };
                let map: HashMap<JobId, InFlightEntry> =
                    entries.into_iter().map(|e| (e.job.id, e)).collect();
                if !map.is_empty() {
                    info!(count = map.len(), "restored in-flight jobs from disk");
                }
                map
            }
            Err(_) => HashMap::new(),
        };

        Self {
            jobs,
            by_id,
            in_flight,
            path,
            in_flight_recovered: false,
            clock,
        }
    }

    /// Read the wheel's clock — primarily for tests and convenience methods.
    pub fn clock(&self) -> &ClockRef { &self.clock }

    /// Drain all jobs whose `next_at` is at or before the wheel's current
    /// clock reading. Convenience wrapper over [`JobWheel::drain_expired`].
    pub fn drain_expired_now(&mut self) -> DrainResult {
        let now = self.clock.now();
        self.drain_expired(now)
    }

    /// Return the next fire time, or `None` if the wheel is empty.
    pub fn next_deadline(&self) -> Option<jiff::Timestamp> {
        self.jobs
            .keys()
            .next()
            .map(|(secs, _)| jiff::Timestamp::from_second(*secs).unwrap())
    }

    /// Pure query: which jobs have expired as of `now`?
    ///
    /// Returns the IDs of all jobs whose `next_at <= now` without mutating
    /// the wheel. Useful when callers need to inspect expired jobs before
    /// committing to side effects.
    pub fn peek_expired(&self, now: Timestamp) -> Vec<JobId> {
        let cutoff: WheelKey = (now.as_second(), Uuid::max());
        self.jobs.range(..=cutoff).map(|(_, e)| e.id).collect()
    }

    /// Move the given jobs from the wheel into the in-flight ledger.
    ///
    /// Jobs whose cron expression yields no future time are separated into
    /// [`DrainResult::cron_expired`] instead of in-flight — they are dead
    /// and will not be re-executed.
    ///
    /// Returns `(fired, cron_expired)` where `fired` contains entries that
    /// were successfully moved to in-flight, and `cron_expired` contains
    /// cron jobs that have no future fire time.
    pub fn mark_fired(&mut self, ids: &[JobId], now: Timestamp) -> (Vec<JobEntry>, Vec<JobEntry>) {
        let mut fired = Vec::new();
        let mut cron_expired = Vec::new();

        for id in ids {
            let Some(key) = self.by_id.remove(id) else {
                continue;
            };
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
            let lease_deadline = now
                .checked_add(jiff::SignedDuration::from_secs(DEFAULT_LEASE_SECS))
                .unwrap_or(now);
            self.in_flight.insert(
                entry.id,
                InFlightEntry {
                    job: entry.clone(),
                    fired_at: now,
                    lease_deadline,
                },
            );
            fired.push(entry);
        }

        (fired, cron_expired)
    }

    /// Reschedule recurring jobs (Interval/Cron) for their next fire time.
    ///
    /// Once-jobs are ignored. Interval jobs use catch-up semantics anchored
    /// on their original `anchor_at` to avoid cumulative drift. Cron jobs
    /// recompute the next fire time from their expression.
    pub fn reschedule_recurring(&mut self, fired: &[JobEntry], now: Timestamp) {
        for entry in fired {
            match &entry.trigger {
                Trigger::Once { .. } => {
                    // One-shot — do not re-insert into the wheel.
                }
                Trigger::Interval {
                    anchor_at,
                    every_secs,
                    next_at,
                } => {
                    // Catch-up semantics: pick the smallest k such that
                    // anchor_at + k * every_secs > now. This eliminates
                    // cumulative drift even after missed periods.
                    //
                    // `anchor_at` should always be Some after `load`'s
                    // backfill pass, but we tolerate None defensively by
                    // falling back to the prior `next_at` as the anchor.
                    let anchor = anchor_at.unwrap_or(*next_at);
                    let every = jiff::SignedDuration::from_secs(*every_secs as i64);
                    let mut next = anchor;
                    while next <= now {
                        match next.checked_add(every) {
                            Ok(t) => next = t,
                            Err(_) => break,
                        }
                    }

                    let mut rescheduled = entry.clone();
                    rescheduled.trigger = Trigger::Interval {
                        anchor_at:  Some(anchor),
                        every_secs: *every_secs,
                        next_at:    next,
                    };
                    let new_key = Self::key(&rescheduled);
                    let id = rescheduled.id;
                    self.jobs.insert(new_key, rescheduled);
                    self.by_id.insert(id, new_key);
                }
                Trigger::Cron { expr, .. } => {
                    // We already verified `next_cron_time` is `Some` in
                    // `mark_fired`, but recompute defensively rather than
                    // threading state.
                    if let Some(next) = Self::next_cron_time(expr, now) {
                        let mut rescheduled = entry.clone();
                        rescheduled.trigger = Trigger::Cron {
                            expr:    expr.clone(),
                            next_at: next,
                        };
                        let new_key = Self::key(&rescheduled);
                        let id = rescheduled.id;
                        self.jobs.insert(new_key, rescheduled);
                        self.by_id.insert(id, new_key);
                    }
                }
            }
        }
    }

    /// Drain all jobs whose `next_at` is at or before `now`.
    ///
    /// Convenience wrapper that calls [`peek_expired`], [`mark_fired`], and
    /// [`reschedule_recurring`] in sequence. This preserves backward
    /// compatibility for callers that don't need the decomposed API.
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
    ///
    /// [`peek_expired`]: Self::peek_expired
    /// [`mark_fired`]: Self::mark_fired
    /// [`reschedule_recurring`]: Self::reschedule_recurring
    pub fn drain_expired(&mut self, now: Timestamp) -> DrainResult {
        let expired_ids = self.peek_expired(now);
        let (fired, cron_expired) = self.mark_fired(&expired_ids, now);
        self.reschedule_recurring(&fired, now);

        info!(
            fired_count = fired.len(),
            cron_expired_count = cron_expired.len(),
            "scheduler drain completed"
        );

        DrainResult {
            fired,
            cron_expired,
        }
    }

    /// Add a job to the wheel.
    pub fn add(&mut self, entry: JobEntry) {
        info!(job_id = %entry.id, trigger = ?entry.trigger, "scheduled job registered");
        let key = Self::key(&entry);
        let id = entry.id;
        self.jobs.insert(key, entry);
        self.by_id.insert(id, key);
    }

    /// Remove a job by ID. Returns the removed entry if found.
    ///
    /// O(1) via the `by_id` sidecar index.
    pub fn remove(&mut self, id: &JobId) -> Option<JobEntry> {
        let key = self.by_id.remove(id)?;
        self.jobs.remove(&key)
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
            info!(job_id = %job_id, "scheduled job completed, removed from in-flight");
            self.persist_in_flight();
        }
        removed
    }

    /// Return in-flight jobs from a previous run for re-firing on startup.
    ///
    /// Only entries whose `lease_deadline` has not yet passed are returned.
    /// Expired entries are logged and discarded — they represent orphaned
    /// executions whose agent crashed or hung beyond the lease window.
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

        let now = self.clock.now();
        let mut expired_ids = Vec::new();
        let mut jobs = Vec::new();

        for (id, entry) in &self.in_flight {
            if entry.lease_deadline > now {
                jobs.push(entry.job.clone());
            } else {
                warn!(
                    job_id = %id,
                    fired_at = %entry.fired_at,
                    lease_deadline = %entry.lease_deadline,
                    "discarding orphaned in-flight job (lease expired)"
                );
                expired_ids.push(*id);
            }
        }

        // Remove expired entries from the ledger and persist.
        if !expired_ids.is_empty() {
            for id in &expired_ids {
                self.in_flight.remove(id);
            }
            self.persist_in_flight();
        }

        if !jobs.is_empty() {
            info!(
                count = jobs.len(),
                "re-firing in-flight jobs from previous run"
            );
        }
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
        let entries: Vec<&InFlightEntry> = self.in_flight.values().collect();
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use jiff::SignedDuration;

    use super::*;
    use crate::{
        identity::{KernelUser, Permission, Principal, Resolved, Role},
        session::SessionKey,
    };

    fn test_principal() -> Principal {
        let user = KernelUser {
            name:        "test-user".into(),
            role:        Role::User,
            permissions: vec![Permission::Spawn],
            enabled:     true,
        };
        Principal::from_user(&user)
    }

    fn make_interval_entry(anchor: Timestamp, every_secs: u64) -> JobEntry {
        JobEntry {
            id:          JobId::new(),
            trigger:     Trigger::Interval {
                anchor_at: Some(anchor),
                every_secs,
                next_at: anchor,
            },
            message:     "tick".into(),
            session_key: SessionKey::new(),
            principal:   test_principal(),
            created_at:  anchor,
            tags:        vec![],
        }
    }

    fn next_interval_at(wheel: &JobWheel, id: JobId) -> Timestamp {
        let entry = wheel
            .jobs
            .values()
            .find(|e| e.id == id)
            .expect("job should still be in wheel after interval reschedule");
        match &entry.trigger {
            Trigger::Interval { next_at, .. } => *next_at,
            other => panic!("expected Interval trigger, got {other:?}"),
        }
    }

    fn make_job(message: &str, trigger: Trigger) -> JobEntry {
        JobEntry {
            id: JobId::new(),
            trigger,
            message: message.into(),
            session_key: SessionKey::new(),
            principal: test_principal(),
            created_at: Timestamp::now(),
            tags: vec![],
        }
    }

    fn future(secs: i64) -> Timestamp {
        Timestamp::now()
            .checked_add(SignedDuration::from_secs(secs))
            .expect("future timestamp should be representable")
    }

    #[test]
    fn by_id_index_stays_in_sync() {
        let tmp = tempfile::tempdir().unwrap();
        let mut wheel = JobWheel::load(tmp.path().join("jobs.json"));

        let job1 = make_job("one", Trigger::Once { run_at: future(60) });
        let id1 = job1.id;
        let job2 = make_job(
            "two",
            Trigger::Interval {
                anchor_at:  None,
                every_secs: 30,
                next_at:    future(30),
            },
        );
        let id2 = job2.id;

        wheel.add(job1);
        wheel.add(job2);

        assert_eq!(wheel.jobs.len(), 2);
        assert_eq!(wheel.by_id.len(), 2);

        let removed = wheel.remove(&id1);
        assert!(removed.is_some());

        assert_eq!(wheel.jobs.len(), 1);
        assert_eq!(wheel.by_id.len(), 1);
        assert!(!wheel.by_id.contains_key(&id1));
        assert!(wheel.by_id.contains_key(&id2));
    }

    #[test]
    fn drain_expired_keeps_by_id_consistent_for_recurring_jobs() {
        let tmp = tempfile::tempdir().unwrap();
        let mut wheel = JobWheel::load(tmp.path().join("jobs.json"));

        let past = Timestamp::now()
            .checked_sub(SignedDuration::from_secs(10))
            .unwrap();

        let once = make_job("once", Trigger::Once { run_at: past });
        let once_id = once.id;
        let interval = make_job(
            "interval",
            Trigger::Interval {
                anchor_at:  None,
                every_secs: 60,
                next_at:    past,
            },
        );
        let interval_id = interval.id;

        wheel.add(once);
        wheel.add(interval);
        assert_eq!(wheel.by_id.len(), 2);

        let result = wheel.drain_expired(Timestamp::now());
        assert_eq!(result.fired.len(), 2);

        // Once job is gone from both maps; interval is rescheduled and
        // present in both with the new key.
        assert_eq!(wheel.jobs.len(), 1);
        assert_eq!(wheel.by_id.len(), 1);
        assert!(!wheel.by_id.contains_key(&once_id));

        let new_key = wheel
            .by_id
            .get(&interval_id)
            .copied()
            .expect("rescheduled interval job must be re-indexed");
        assert!(wheel.jobs.contains_key(&new_key));
    }

    #[test]
    fn load_rebuilds_by_id_from_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("jobs.json");

        let id1 = {
            let mut wheel = JobWheel::load(path.clone());
            let job = make_job(
                "persisted",
                Trigger::Once {
                    run_at: future(120),
                },
            );
            let id = job.id;
            wheel.add(job);
            wheel.persist();
            id
        };

        let reloaded = JobWheel::load(path);
        assert_eq!(reloaded.jobs.len(), 1);
        assert_eq!(reloaded.by_id.len(), 1);
        assert!(reloaded.by_id.contains_key(&id1));
    }

    /// Verify that `JobEntry` round-trips through JSON with a typed
    /// `Principal<Resolved>`. The `Resolved` marker is `PhantomData` and
    /// must not affect the serialized representation.
    #[test]
    fn job_entry_principal_roundtrip() {
        let user = KernelUser {
            name:        "alice".into(),
            role:        Role::User,
            permissions: vec![Permission::Spawn],
            enabled:     true,
        };
        let principal: Principal<Resolved> = Principal::from_user(&user);

        let job = JobEntry {
            id:          JobId::new(),
            trigger:     Trigger::Once {
                run_at: Timestamp::from_second(1_700_000_000).unwrap(),
            },
            message:     "hello".into(),
            session_key: SessionKey::new(),
            principal:   principal.clone(),
            created_at:  Timestamp::from_second(1_699_000_000).unwrap(),
            tags:        vec!["tag1".into()],
        };

        let json = serde_json::to_string(&job).expect("serialize JobEntry");
        let back: JobEntry = serde_json::from_str(&json).expect("deserialize JobEntry");
        assert_eq!(job.principal, back.principal);
        assert_eq!(job.id, back.id);
        assert_eq!(job.message, back.message);
        assert_eq!(job.tags, back.tags);
    }

    /// Reschedule lands on `anchor + N*period`, not on `now + period`.
    /// Anchor at t0, period 10s, drain at t0+25s → next_at must be t0+30s.
    #[test]
    fn interval_reschedule_is_catch_up() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("jobs.json");
        let mut wheel = JobWheel::load(path);

        let t0 = Timestamp::from_second(1_700_000_000).unwrap();
        let entry = make_interval_entry(t0, 10);
        let id = entry.id;
        wheel.add(entry);

        let now = t0.checked_add(SignedDuration::from_secs(25)).unwrap();
        let result = wheel.drain_expired(now);
        assert_eq!(result.fired.len(), 1);

        let expected = t0.checked_add(SignedDuration::from_secs(30)).unwrap();
        assert_eq!(next_interval_at(&wheel, id), expected);
    }

    /// Multiple missed periods skip ahead in one shot — no slow catch-up loop
    /// of N drains, no compound drift. Anchor at t0, period 10s, drain at
    /// t0+100s → next_at must be t0+110s.
    #[test]
    fn interval_multiple_missed_periods_skip_ahead() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("jobs.json");
        let mut wheel = JobWheel::load(path);

        let t0 = Timestamp::from_second(1_700_000_000).unwrap();
        let entry = make_interval_entry(t0, 10);
        let id = entry.id;
        wheel.add(entry);

        let now = t0.checked_add(SignedDuration::from_secs(100)).unwrap();
        let result = wheel.drain_expired(now);
        assert_eq!(result.fired.len(), 1);

        let expected = t0.checked_add(SignedDuration::from_secs(110)).unwrap();
        assert_eq!(next_interval_at(&wheel, id), expected);
    }

    /// Legacy `jobs.json` entries serialized before `anchor_at` existed must
    /// be backfilled at load time so subsequent reschedules use catch-up
    /// arithmetic anchored on the prior `next_at`.
    #[test]
    fn legacy_interval_without_anchor_backfills() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("jobs.json");

        // Persist a normal entry, then strip the `anchor_at` field from the
        // on-disk JSON so the file matches what an older kernel would have
        // written. This is more robust than hand-crafting JSON whose shape
        // would drift if neighbouring fields change.
        {
            let mut wheel = JobWheel::load(path.clone());
            let t0 = Timestamp::from_second(1_700_000_000).unwrap();
            wheel.add(make_interval_entry(t0, 10));
            wheel.persist();
        }

        let raw = std::fs::read_to_string(&path).unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        for entry in value.as_array_mut().unwrap() {
            let trigger = entry.get_mut("trigger").unwrap().as_object_mut().unwrap();
            assert!(trigger.remove("anchor_at").is_some());
        }
        std::fs::write(&path, serde_json::to_string(&value).unwrap()).unwrap();

        let wheel = JobWheel::load(path);
        let entry = wheel
            .jobs
            .values()
            .next()
            .expect("legacy entry should load");
        match &entry.trigger {
            Trigger::Interval {
                anchor_at,
                every_secs,
                next_at,
            } => {
                assert_eq!(*every_secs, 10);
                assert_eq!(
                    *anchor_at,
                    Some(*next_at),
                    "legacy interval should backfill anchor_at = next_at",
                );
            }
            other => panic!("expected Interval, got {other:?}"),
        }
    }

    fn t0() -> Timestamp {
        // Fixed reference instant: 2030-01-01T00:00:00Z.
        Timestamp::from_second(1_893_456_000).unwrap()
    }

    fn make_entry(run_at: Timestamp) -> JobEntry {
        JobEntry {
            id:          JobId::new(),
            trigger:     Trigger::Once { run_at },
            message:     "test".to_string(),
            session_key: SessionKey::new(),
            principal:   test_principal(),
            created_at:  run_at,
            tags:        Vec::new(),
        }
    }

    #[test]
    fn fake_clock_advance_and_set() {
        let clock = FakeClock::new(t0());
        assert_eq!(clock.now(), t0());

        clock.advance(std::time::Duration::from_secs(30));
        assert_eq!(clock.now().as_second(), t0().as_second() + 30);

        let target = Timestamp::from_second(t0().as_second() + 1_000).unwrap();
        clock.set(target);
        assert_eq!(clock.now(), target);
    }

    #[test]
    fn drain_uses_clock_time() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("jobs.json");
        let clock = Arc::new(FakeClock::new(t0()));
        let mut wheel = JobWheel::load_with_clock(path, clock.clone());

        let run_at = Timestamp::from_second(t0().as_second() + 60).unwrap();
        wheel.add(make_entry(run_at));

        // At t0 nothing should fire — the job runs 60s in the future.
        let result = wheel.drain_expired_now();
        assert!(
            result.fired.is_empty(),
            "expected no jobs at t0, got {}",
            result.fired.len()
        );

        // Advance past the deadline and drain again.
        clock.advance(std::time::Duration::from_secs(61));
        let result = wheel.drain_expired_now();
        assert_eq!(
            result.fired.len(),
            1,
            "expected one job after advancing 61s"
        );
    }

    /// In-flight entries within the lease window are re-fired on recovery.
    #[test]
    fn take_in_flight_returns_jobs_within_lease() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("jobs.json");
        let clock = Arc::new(FakeClock::new(t0()));
        let mut wheel = JobWheel::load_with_clock(path.clone(), clock.clone());

        // Add a job in the past, drain it into in-flight.
        let run_at = Timestamp::from_second(t0().as_second() - 10).unwrap();
        wheel.add(make_entry(run_at));
        let result = wheel.drain_expired_now();
        assert_eq!(result.fired.len(), 1);

        // Persist and reload — simulates a restart.
        wheel.persist();
        let mut reloaded = JobWheel::load_with_clock(path, clock.clone());

        // Clock is still at t0 — well within the 300s lease.
        let recovered = reloaded.take_in_flight();
        assert_eq!(recovered.len(), 1, "job within lease should be re-fired");
    }

    /// In-flight entries past the lease deadline are discarded on recovery.
    #[test]
    fn take_in_flight_discards_expired_lease() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("jobs.json");
        let clock = Arc::new(FakeClock::new(t0()));
        let mut wheel = JobWheel::load_with_clock(path.clone(), clock.clone());

        let run_at = Timestamp::from_second(t0().as_second() - 10).unwrap();
        wheel.add(make_entry(run_at));
        let result = wheel.drain_expired_now();
        assert_eq!(result.fired.len(), 1);

        wheel.persist();

        // Advance clock past the lease deadline (300s + margin).
        clock.advance(std::time::Duration::from_secs(400));
        let mut reloaded = JobWheel::load_with_clock(path, clock.clone());

        let recovered = reloaded.take_in_flight();
        assert!(
            recovered.is_empty(),
            "expired-lease job should be discarded, got {}",
            recovered.len()
        );
    }

    /// InFlightEntry round-trips through JSON.
    #[test]
    fn in_flight_entry_serde_roundtrip() {
        let job = make_entry(t0());
        let entry = InFlightEntry {
            job:            job.clone(),
            fired_at:       t0(),
            lease_deadline: Timestamp::from_second(t0().as_second() + DEFAULT_LEASE_SECS).unwrap(),
        };
        let json = serde_json::to_string(&entry).expect("serialize InFlightEntry");
        let back: InFlightEntry = serde_json::from_str(&json).expect("deserialize InFlightEntry");
        assert_eq!(back.job.id, job.id);
        assert_eq!(back.fired_at, t0());
        assert_eq!(
            back.lease_deadline.as_second(),
            t0().as_second() + DEFAULT_LEASE_SECS
        );
    }
}
