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

//! Persistent scheduler for agent jobs backed by a local JSON file.

use std::{path::PathBuf, str::FromStr};

use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};
use tracing::warn;

/// A scheduled job that the agent should execute when its trigger fires.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentJob {
    /// Unique identifier (ULID).
    pub id:          String,
    /// Natural-language intent sent as the user prompt when the job fires.
    pub message:     String,
    /// When / how often the job should fire.
    pub trigger:     AgentTrigger,
    /// Session key for conversation context (default `"agent:proactive"`).
    pub session_key: String,
    /// When the job was created.
    pub created_at:  jiff::Timestamp,
    /// When the job last executed (if ever).
    pub last_run_at: Option<jiff::Timestamp>,
    /// Whether the job is active.
    pub enabled:     bool,
}

/// Trigger strategy for an [`AgentJob`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentTrigger {
    /// Fires according to a cron expression (5-field format).
    Cron { expr: String },
    /// Fires once at the specified absolute time, then is removed.
    Delay { run_at: jiff::Timestamp },
    /// Fires repeatedly at a fixed interval.
    Interval { seconds: u64 },
}

/// File-backed scheduler that persists agent jobs as JSON.
pub struct AgentScheduler {
    jobs_path:  PathBuf,
    jobs:       RwLock<Vec<AgentJob>>,
    save_mutex: Mutex<()>,
}

impl AgentScheduler {
    /// Create a new scheduler backed by the given JSON file path.
    pub fn new(jobs_path: PathBuf) -> Self {
        Self {
            jobs_path,
            jobs: RwLock::new(Vec::new()),
            save_mutex: Mutex::new(()),
        }
    }

    /// Load jobs from the backing JSON file. Tolerates missing files.
    ///
    /// If the main file is corrupted, the loader tries the `.json.tmp`
    /// backup. If that also fails, the corrupted file is renamed to
    /// `.json.corrupted` and the scheduler starts with an empty job list
    /// (graceful degradation).
    ///
    /// If no jobs exist after loading (file missing or empty), a default
    /// daily diary cron job is seeded so the agent writes a diary entry
    /// every evening at 22:00.
    pub async fn load(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let path = &self.jobs_path;
        let tmp_path = self.jobs_path.with_extension("json.tmp");

        if path.exists() {
            match Self::try_load_from(path).await {
                Ok(loaded) => {
                    let mut jobs = self.jobs.write().await;
                    *jobs = loaded;
                }
                Err(main_err) => {
                    warn!(
                        path = %path.display(),
                        error = %main_err,
                        "failed to parse jobs file, attempting .tmp backup"
                    );

                    // Try the .tmp file as a backup.
                    if tmp_path.exists() {
                        match Self::try_load_from(&tmp_path).await {
                            Ok(loaded) => {
                                warn!(
                                    path = %tmp_path.display(),
                                    "recovered jobs from .tmp backup"
                                );
                                let mut jobs = self.jobs.write().await;
                                *jobs = loaded;
                            }
                            Err(tmp_err) => {
                                warn!(
                                    path = %tmp_path.display(),
                                    error = %tmp_err,
                                    "backup .tmp file also corrupted, starting empty"
                                );
                            }
                        }
                    }

                    // Rename the corrupted main file for investigation.
                    let corrupted_path = self.jobs_path.with_extension("json.corrupted");
                    if let Err(e) = tokio::fs::rename(path, &corrupted_path).await {
                        warn!(
                            error = %e,
                            "failed to rename corrupted file to .corrupted"
                        );
                    }
                }
            }
        } else if tmp_path.exists() {
            // Main file missing but .tmp exists — recover from it.
            match Self::try_load_from(&tmp_path).await {
                Ok(loaded) => {
                    warn!(
                        path = %tmp_path.display(),
                        "main file missing, recovered jobs from .tmp backup"
                    );
                    let mut jobs = self.jobs.write().await;
                    *jobs = loaded;
                }
                Err(e) => {
                    warn!(
                        path = %tmp_path.display(),
                        error = %e,
                        "main file missing and .tmp backup corrupted, starting empty"
                    );
                }
            }
        }

        // Seed a default daily-diary job when no jobs exist.
        let needs_seed = self.jobs.read().await.is_empty();
        if needs_seed {
            let diary_job = AgentJob {
                id:          ulid::Ulid::new().to_string(),
                message:     "写今天的日记。回顾今天的用户活动和你的工作，写一篇日记到 \
                              docs/src/diary/ 目录。"
                    .to_string(),
                trigger:     AgentTrigger::Cron {
                    expr: "0 22 * * *".to_string(),
                },
                session_key: "agent:proactive".to_string(),
                created_at:  jiff::Timestamp::now(),
                last_run_at: None,
                enabled:     true,
            };
            let mut jobs = self.jobs.write().await;
            jobs.push(diary_job);
            drop(jobs);
            self.save().await?;
        }
        Ok(())
    }

    /// Persist current jobs to the backing JSON file using atomic
    /// write-then-rename to prevent corruption from partial writes.
    pub async fn save(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _guard = self.save_mutex.lock().await;

        let jobs = self.jobs.read().await;
        let data = serde_json::to_string_pretty(&*jobs)?;
        drop(jobs);

        // Ensure parent directory exists.
        if let Some(parent) = self.jobs_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // Write to temp file first.
        let tmp_path = self.jobs_path.with_extension("json.tmp");
        tokio::fs::write(&tmp_path, &data).await?;

        // Atomic rename (POSIX guarantees atomicity for same-directory rename).
        tokio::fs::rename(&tmp_path, &self.jobs_path).await?;

        Ok(())
    }

    /// Add a job and persist.
    pub async fn add(&self, job: AgentJob) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        {
            let mut jobs = self.jobs.write().await;
            jobs.push(job);
        }
        self.save().await
    }

    /// Remove a job by ID. Returns `true` if a job was removed.
    pub async fn remove(&self, id: &str) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let removed = {
            let mut jobs = self.jobs.write().await;
            let before = jobs.len();
            jobs.retain(|j| j.id != id);
            jobs.len() < before
        };
        if removed {
            self.save().await?;
        }
        Ok(removed)
    }

    /// List all jobs (enabled and disabled).
    pub async fn list(&self) -> Vec<AgentJob> { self.jobs.read().await.clone() }

    /// Return all enabled jobs whose triggers indicate they should run now.
    pub async fn get_due_jobs(&self) -> Vec<AgentJob> {
        let jobs = self.jobs.read().await;
        let now = jiff::Timestamp::now();

        jobs.iter()
            .filter(|j| j.enabled && Self::is_due(j, now))
            .cloned()
            .collect()
    }

    /// Mark a job as executed: update `last_run_at`, remove if `Delay`, and
    /// persist.
    pub async fn mark_executed(
        &self,
        id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let now = jiff::Timestamp::now();
        {
            let mut jobs = self.jobs.write().await;
            // Update last_run_at on the job, then remove if it was a Delay.
            if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
                job.last_run_at = Some(now);
                if matches!(job.trigger, AgentTrigger::Delay { .. }) {
                    // Mark for removal after loop.
                }
            }
            jobs.retain(|j| !(j.id == id && matches!(j.trigger, AgentTrigger::Delay { .. })));
        }
        self.save().await
    }

    /// Try to read and deserialize jobs from the given path.
    async fn try_load_from(
        path: &std::path::Path,
    ) -> Result<Vec<AgentJob>, Box<dyn std::error::Error + Send + Sync>> {
        let data = tokio::fs::read_to_string(path).await?;
        let loaded: Vec<AgentJob> = serde_json::from_str(&data)?;
        Ok(loaded)
    }

    /// Check whether a single job is due at the given instant.
    fn is_due(job: &AgentJob, now: jiff::Timestamp) -> bool {
        match &job.trigger {
            AgentTrigger::Cron { expr } => Self::is_cron_due(expr, job.last_run_at, now),
            AgentTrigger::Delay { run_at } => *run_at <= now,
            AgentTrigger::Interval { seconds } => match job.last_run_at {
                None => true,
                Some(last) => {
                    let elapsed_secs = now.as_second() - last.as_second();
                    #[expect(clippy::cast_possible_wrap)]
                    let interval_secs = *seconds as i64;
                    elapsed_secs >= interval_secs
                }
            },
        }
    }

    /// Forward-looking cron check that uses `last_run_at` to prevent
    /// duplicate execution.
    ///
    /// Finds the next cron occurrence after `last_run_at` (or epoch if never
    /// run) and returns `true` when that occurrence falls at or before `now`.
    /// This is safe regardless of how long the worker poll interval is — a
    /// delayed poll simply fires the missed trigger on the next tick, and
    /// once `mark_executed` updates `last_run_at` the same trigger will not
    /// fire again.
    fn is_cron_due(expr: &str, last_run_at: Option<jiff::Timestamp>, now: jiff::Timestamp) -> bool {
        let Ok(cron) = croner::Cron::from_str(expr) else {
            warn!(expr, "invalid cron expression in agent job");
            return false;
        };

        // Anchor: find the next occurrence after last_run_at (or epoch).
        let anchor = last_run_at.map(|ts| ts.as_second()).unwrap_or(0);
        let anchor_chrono =
            chrono::DateTime::from_timestamp(anchor, 0).unwrap_or_else(chrono::Utc::now);

        let now_chrono =
            chrono::DateTime::from_timestamp(now.as_second(), 0).unwrap_or_else(chrono::Utc::now);

        // Find the first occurrence strictly after the anchor.
        cron.find_next_occurrence(&anchor_chrono, false)
            .is_ok_and(|next| next <= now_chrono)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn add_list_remove() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.json");
        let scheduler = AgentScheduler::new(path.clone());

        let job = AgentJob {
            id:          "test-1".to_owned(),
            message:     "hello".to_owned(),
            trigger:     AgentTrigger::Interval { seconds: 300 },
            session_key: "agent:proactive".to_owned(),
            created_at:  jiff::Timestamp::now(),
            last_run_at: None,
            enabled:     true,
        };

        scheduler.add(job).await.unwrap();
        assert_eq!(scheduler.list().await.len(), 1);

        // Verify file was written.
        assert!(path.exists());

        // Remove.
        let removed = scheduler.remove("test-1").await.unwrap();
        assert!(removed);
        assert!(scheduler.list().await.is_empty());

        // Remove non-existent.
        let removed = scheduler.remove("nope").await.unwrap();
        assert!(!removed);
    }

    #[tokio::test]
    async fn load_persisted_jobs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.json");

        // Create and save.
        {
            let scheduler = AgentScheduler::new(path.clone());
            scheduler
                .add(AgentJob {
                    id:          "persist-1".to_owned(),
                    message:     "check in".to_owned(),
                    trigger:     AgentTrigger::Interval { seconds: 600 },
                    session_key: "agent:proactive".to_owned(),
                    created_at:  jiff::Timestamp::now(),
                    last_run_at: None,
                    enabled:     true,
                })
                .await
                .unwrap();
        }

        // Load from new instance.
        let scheduler2 = AgentScheduler::new(path);
        scheduler2.load().await.unwrap();
        let jobs = scheduler2.list().await;
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].id, "persist-1");
    }

    #[tokio::test]
    async fn load_missing_file_seeds_default_job() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.json");
        let scheduler = AgentScheduler::new(path);
        scheduler.load().await.unwrap();
        // When no file exists, load() seeds a default daily-diary job.
        assert_eq!(scheduler.list().await.len(), 1);
    }

    #[tokio::test]
    async fn delay_job_is_due_and_removed_after_execution() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.json");
        let scheduler = AgentScheduler::new(path);

        let past = jiff::Timestamp::now() - std::time::Duration::from_secs(10);
        scheduler
            .add(AgentJob {
                id:          "delay-1".to_owned(),
                message:     "one-shot".to_owned(),
                trigger:     AgentTrigger::Delay { run_at: past },
                session_key: "agent:proactive".to_owned(),
                created_at:  jiff::Timestamp::now(),
                last_run_at: None,
                enabled:     true,
            })
            .await
            .unwrap();

        let due = scheduler.get_due_jobs().await;
        assert_eq!(due.len(), 1);

        scheduler.mark_executed("delay-1").await.unwrap();
        // Delay job should be removed after execution.
        assert!(scheduler.list().await.is_empty());
    }

    #[tokio::test]
    async fn interval_job_becomes_due() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.json");
        let scheduler = AgentScheduler::new(path);

        // Job with last_run_at long ago should be due.
        let long_ago = jiff::Timestamp::now() - std::time::Duration::from_secs(600);
        scheduler
            .add(AgentJob {
                id:          "interval-1".to_owned(),
                message:     "repeat".to_owned(),
                trigger:     AgentTrigger::Interval { seconds: 300 },
                session_key: "agent:proactive".to_owned(),
                created_at:  jiff::Timestamp::now(),
                last_run_at: Some(long_ago),
                enabled:     true,
            })
            .await
            .unwrap();

        let due = scheduler.get_due_jobs().await;
        assert_eq!(due.len(), 1);

        // After execution, job remains (interval type).
        scheduler.mark_executed("interval-1").await.unwrap();
        assert_eq!(scheduler.list().await.len(), 1);
    }

    #[tokio::test]
    async fn disabled_job_not_due() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.json");
        let scheduler = AgentScheduler::new(path);

        scheduler
            .add(AgentJob {
                id:          "disabled-1".to_owned(),
                message:     "should not fire".to_owned(),
                trigger:     AgentTrigger::Interval { seconds: 1 },
                session_key: "agent:proactive".to_owned(),
                created_at:  jiff::Timestamp::now(),
                last_run_at: None,
                enabled:     false,
            })
            .await
            .unwrap();

        let due = scheduler.get_due_jobs().await;
        assert!(due.is_empty());
    }

    #[tokio::test]
    async fn save_uses_atomic_rename() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.json");
        let scheduler = AgentScheduler::new(path.clone());

        scheduler
            .add(AgentJob {
                id:          "atomic-1".to_owned(),
                message:     "atomic test".to_owned(),
                trigger:     AgentTrigger::Interval { seconds: 60 },
                session_key: "agent:proactive".to_owned(),
                created_at:  jiff::Timestamp::now(),
                last_run_at: None,
                enabled:     true,
            })
            .await
            .unwrap();

        assert!(path.exists());
        let tmp_path = path.with_extension("json.tmp");
        assert!(!tmp_path.exists());
    }

    #[tokio::test]
    async fn load_corrupted_file_recovers_gracefully() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.json");

        tokio::fs::write(&path, "NOT VALID JSON{{{").await.unwrap();

        let scheduler = AgentScheduler::new(path.clone());
        scheduler.load().await.unwrap();

        let corrupted_path = path.with_extension("json.corrupted");
        assert!(corrupted_path.exists());
        assert!(path.exists());
        assert_eq!(scheduler.list().await.len(), 1);
    }

    #[tokio::test]
    async fn load_corrupted_file_recovers_from_tmp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.json");
        let tmp_path = path.with_extension("json.tmp");

        {
            let scheduler = AgentScheduler::new(path.clone());
            scheduler
                .add(AgentJob {
                    id:          "backup-1".to_owned(),
                    message:     "backed up".to_owned(),
                    trigger:     AgentTrigger::Interval { seconds: 120 },
                    session_key: "agent:proactive".to_owned(),
                    created_at:  jiff::Timestamp::now(),
                    last_run_at: None,
                    enabled:     true,
                })
                .await
                .unwrap();
            tokio::fs::copy(&path, &tmp_path).await.unwrap();
        }

        tokio::fs::write(&path, "CORRUPTED!!!").await.unwrap();

        let scheduler = AgentScheduler::new(path.clone());
        scheduler.load().await.unwrap();
        let jobs = scheduler.list().await;
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].id, "backup-1");
    }

    #[tokio::test]
    async fn load_recovers_from_tmp_when_main_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.json");
        let tmp_path = path.with_extension("json.tmp");

        {
            let scheduler = AgentScheduler::new(path.clone());
            scheduler
                .add(AgentJob {
                    id:          "tmp-only-1".to_owned(),
                    message:     "from tmp".to_owned(),
                    trigger:     AgentTrigger::Interval { seconds: 60 },
                    session_key: "agent:proactive".to_owned(),
                    created_at:  jiff::Timestamp::now(),
                    last_run_at: None,
                    enabled:     true,
                })
                .await
                .unwrap();
            tokio::fs::copy(&path, &tmp_path).await.unwrap();
            tokio::fs::remove_file(&path).await.unwrap();
        }

        assert!(!path.exists());
        assert!(tmp_path.exists());

        let scheduler = AgentScheduler::new(path);
        scheduler.load().await.unwrap();
        let jobs = scheduler.list().await;
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].id, "tmp-only-1");
    }

    // -- is_cron_due unit tests ------------------------------------------------

    #[test]
    fn cron_due_never_run_fires_if_past() {
        let now = jiff::Timestamp::now();
        assert!(AgentScheduler::is_cron_due("* * * * *", None, now));
    }

    #[test]
    fn cron_due_recently_run_does_not_fire_again() {
        let now = jiff::Timestamp::now();
        let ten_secs_ago = now - std::time::Duration::from_secs(10);
        assert!(!AgentScheduler::is_cron_due(
            "* * * * *",
            Some(ten_secs_ago),
            now
        ));
    }

    #[test]
    fn cron_due_fires_after_interval_elapsed() {
        let now = jiff::Timestamp::now();
        let seventy_secs_ago = now - std::time::Duration::from_secs(70);
        assert!(AgentScheduler::is_cron_due(
            "* * * * *",
            Some(seventy_secs_ago),
            now
        ));
    }

    #[test]
    fn cron_invalid_expression() {
        let now = jiff::Timestamp::now();
        assert!(!AgentScheduler::is_cron_due("not a cron", None, now));
    }
}
