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

use std::{collections::BTreeMap, path::PathBuf};

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
pub struct JobWheel {
    /// Jobs ordered by (next_fire_time_secs, job_uuid).
    jobs: BTreeMap<WheelKey, JobEntry>,
    /// Path to the JSON persistence file.
    path: PathBuf,
}

impl JobWheel {
    /// Build a wheel key from a job entry.
    fn key(entry: &JobEntry) -> WheelKey { (entry.trigger.next_at().as_second(), entry.id.0) }

    /// Load jobs from the JSON persistence file, or create an empty wheel.
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
        Self { jobs, path }
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
    /// - `Once` jobs are removed permanently.
    /// - `Interval` jobs have their `next_at` advanced and are re-inserted.
    /// - `Cron` jobs compute the next fire time from their expression and are
    ///   re-inserted. If the cron expression yields no future time, the job is
    ///   removed.
    pub fn drain_expired(&mut self, now: Timestamp) -> Vec<JobEntry> {
        let mut expired = Vec::new();
        let cutoff: WheelKey = (now.as_second(), Uuid::max());

        // Collect all keys up to `now`.
        let keys: Vec<WheelKey> = self.jobs.range(..=cutoff).map(|(k, _)| *k).collect();

        for key in keys {
            if let Some(entry) = self.jobs.remove(&key) {
                expired.push(entry.clone());

                // Re-schedule recurring jobs.
                match entry.trigger.clone() {
                    Trigger::Once { .. } => {
                        // One-shot — do not re-insert.
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
                    Trigger::Cron { expr, .. } => match Self::next_cron_time(&expr, now) {
                        Some(next) => {
                            let mut rescheduled = entry;
                            rescheduled.trigger = Trigger::Cron {
                                expr,
                                next_at: next,
                            };
                            self.jobs.insert(Self::key(&rescheduled), rescheduled);
                        }
                        None => {
                            warn!(job_id = %entry.id, expr = expr, "cron expression yields no future time, removing job");
                        }
                    },
                }
            }
        }

        expired
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

    /// Persist the current state to the JSON file.
    pub fn persist(&self) {
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

    /// Compute the next fire time for a cron expression after `after`.
    fn next_cron_time(expr: &str, after: Timestamp) -> Option<Timestamp> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Principal;

    /// Helper to create a job entry with the given trigger.
    fn make_job(trigger: Trigger) -> JobEntry {
        JobEntry {
            id: JobId::default(),
            trigger,
            message: "test task".to_string(),
            session_key: SessionKey::default(),
            principal: Principal::lookup("test-user".to_string()),
            created_at: Timestamp::now(),
        }
    }

    /// Helper to create a JobWheel backed by a temp file.
    fn temp_wheel() -> JobWheel {
        let path =
            std::env::temp_dir().join(format!("rara-test-jobs-{}.json", uuid::Uuid::new_v4()));
        JobWheel {
            jobs: BTreeMap::new(),
            path,
        }
    }

    #[test]
    fn drain_expired_removes_once_jobs() {
        let mut wheel = temp_wheel();
        let past = Timestamp::now()
            .checked_sub(jiff::SignedDuration::from_secs(10))
            .unwrap();
        let job = make_job(Trigger::Once { run_at: past });
        let job_id = job.id;
        wheel.add(job);

        let expired = wheel.drain_expired(Timestamp::now());
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].id, job_id);

        // Once job should NOT be re-inserted.
        assert!(wheel.list(None).is_empty());
    }

    #[test]
    fn drain_expired_reschedules_interval_jobs() {
        let mut wheel = temp_wheel();
        let past = Timestamp::now()
            .checked_sub(jiff::SignedDuration::from_secs(10))
            .unwrap();
        let job = make_job(Trigger::Interval {
            every_secs: 60,
            next_at:    past,
        });
        let job_id = job.id;
        wheel.add(job);

        let expired = wheel.drain_expired(Timestamp::now());
        assert_eq!(expired.len(), 1);

        // Interval job should be re-inserted with updated next_at.
        let remaining = wheel.list(None);
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, job_id);
        assert!(remaining[0].trigger.next_at() > Timestamp::now());
    }

    #[test]
    fn requeue_once_job_after_drain() {
        // Simulate the scenario where a Once job is drained but cannot be
        // spawned (parent at child limit).  The job should be requeued with
        // a short delay so it is not lost.
        let mut wheel = temp_wheel();
        let past = Timestamp::now()
            .checked_sub(jiff::SignedDuration::from_secs(10))
            .unwrap();
        let job = make_job(Trigger::Once { run_at: past });
        let job_id = job.id;
        wheel.add(job);

        // drain_expired consumes the Once job.
        let expired = wheel.drain_expired(Timestamp::now());
        assert_eq!(expired.len(), 1);
        assert!(wheel.list(None).is_empty(), "Once job should be consumed");

        // Requeue: simulate what requeue_job() does.
        let mut requeued = expired.into_iter().next().unwrap();
        let retry_at = Timestamp::now()
            .checked_add(jiff::SignedDuration::from_secs(10))
            .unwrap();
        requeued.trigger = Trigger::Once { run_at: retry_at };
        wheel.add(requeued);

        // The job should be back in the wheel.
        let remaining = wheel.list(None);
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, job_id);
        assert!(
            remaining[0].trigger.next_at() > Timestamp::now(),
            "requeued job should fire in the future"
        );
    }

    #[test]
    fn recurring_job_not_requeued_after_drain() {
        // Interval/Cron jobs are already rescheduled by drain_expired.
        // Requeueing them would create an extra duplicate run.
        let mut wheel = temp_wheel();
        let past = Timestamp::now()
            .checked_sub(jiff::SignedDuration::from_secs(10))
            .unwrap();
        let job = make_job(Trigger::Interval {
            every_secs: 60,
            next_at:    past,
        });
        wheel.add(job);

        let expired = wheel.drain_expired(Timestamp::now());
        assert_eq!(expired.len(), 1);

        // drain_expired already rescheduled the interval job.
        assert_eq!(
            wheel.list(None).len(),
            1,
            "interval job should be rescheduled"
        );

        // Verify that the expired entry is an Interval trigger — the
        // requeue_job() method should NOT requeue it (early return).
        assert!(
            matches!(expired[0].trigger, Trigger::Interval { .. }),
            "expired entry should retain Interval trigger"
        );
    }

    #[test]
    fn add_and_remove_job() {
        let mut wheel = temp_wheel();
        let job = make_job(Trigger::Once {
            run_at: Timestamp::now(),
        });
        let job_id = job.id;
        wheel.add(job);

        assert_eq!(wheel.list(None).len(), 1);
        let removed = wheel.remove(&job_id);
        assert!(removed.is_some());
        assert!(wheel.list(None).is_empty());
    }

    #[test]
    fn next_deadline_returns_earliest() {
        let mut wheel = temp_wheel();
        let t1 = Timestamp::now()
            .checked_add(jiff::SignedDuration::from_secs(100))
            .unwrap();
        let t2 = Timestamp::now()
            .checked_add(jiff::SignedDuration::from_secs(200))
            .unwrap();

        wheel.add(make_job(Trigger::Once { run_at: t2 }));
        wheel.add(make_job(Trigger::Once { run_at: t1 }));

        let deadline = wheel.next_deadline().unwrap();
        // next_deadline truncates to seconds, so compare at second granularity.
        assert_eq!(deadline.as_second(), t1.as_second());
    }

    #[test]
    fn list_filters_by_session() {
        let mut wheel = temp_wheel();
        let sk1 = SessionKey::default();
        let sk2 = SessionKey::default();

        let mut job1 = make_job(Trigger::Once {
            run_at: Timestamp::now(),
        });
        job1.session_key = sk1;
        let mut job2 = make_job(Trigger::Once {
            run_at: Timestamp::now(),
        });
        job2.session_key = sk2;

        wheel.add(job1);
        wheel.add(job2);

        assert_eq!(wheel.list(Some(&sk1)).len(), 1);
        assert_eq!(wheel.list(Some(&sk2)).len(), 1);
        assert_eq!(wheel.list(None).len(), 2);
    }
}
