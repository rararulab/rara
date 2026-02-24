use std::{
    collections::VecDeque,
    sync::atomic::{AtomicU64, Ordering},
};

use async_trait::async_trait;
use serde::Serialize;
use tokio::sync::RwLock;

use super::types::{AgentTaskKind, TaskRecord, TaskStatus};

/// Filter for querying the log store.
pub struct LogFilter {
    pub limit: usize,
    pub kind: Option<AgentTaskKind>,
    pub status: Option<TaskStatus>,
    pub since: Option<jiff::Timestamp>,
}

/// Aggregate statistics about dispatcher activity.
#[derive(Debug, Clone, Serialize)]
pub struct DispatcherStats {
    pub total_submitted: u64,
    pub total_completed: u64,
    pub total_errors: u64,
    pub total_deduped: u64,
    pub total_cancelled: u64,
    pub uptime_seconds: u64,
}

/// Pluggable storage for task execution records.
#[async_trait]
pub trait DispatcherLogStore: Send + Sync + 'static {
    async fn append(&self, record: TaskRecord);
    async fn query(&self, filter: LogFilter) -> Vec<TaskRecord>;
    async fn stats(&self) -> DispatcherStats;
}

/// In-memory ring buffer implementation of [`DispatcherLogStore`].
pub struct InMemoryLogStore {
    records: RwLock<VecDeque<TaskRecord>>,
    capacity: usize,
    submitted: AtomicU64,
    completed: AtomicU64,
    errors: AtomicU64,
    deduped: AtomicU64,
    cancelled: AtomicU64,
    started_at: jiff::Timestamp,
}

impl InMemoryLogStore {
    pub fn new(capacity: usize) -> Self {
        Self {
            records: RwLock::new(VecDeque::with_capacity(capacity)),
            capacity,
            submitted: AtomicU64::new(0),
            completed: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            deduped: AtomicU64::new(0),
            cancelled: AtomicU64::new(0),
            started_at: jiff::Timestamp::now(),
        }
    }
}

#[async_trait]
impl DispatcherLogStore for InMemoryLogStore {
    async fn append(&self, record: TaskRecord) {
        // Update counters based on status.
        match &record.status {
            TaskStatus::Completed => {
                self.completed.fetch_add(1, Ordering::Relaxed);
            }
            TaskStatus::Error => {
                self.errors.fetch_add(1, Ordering::Relaxed);
            }
            TaskStatus::Deduped => {
                self.deduped.fetch_add(1, Ordering::Relaxed);
            }
            TaskStatus::Cancelled => {
                self.cancelled.fetch_add(1, Ordering::Relaxed);
            }
            TaskStatus::Queued => {
                self.submitted.fetch_add(1, Ordering::Relaxed);
            }
            TaskStatus::Running => {}
        }

        let mut records = self.records.write().await;
        if records.len() >= self.capacity {
            records.pop_front();
        }
        records.push_back(record);
    }

    async fn query(&self, filter: LogFilter) -> Vec<TaskRecord> {
        let records = self.records.read().await;
        records
            .iter()
            .rev()
            .filter(|r| {
                if let Some(ref kind) = filter.kind {
                    if r.kind.label() != kind.label() {
                        return false;
                    }
                }
                if let Some(ref status) = filter.status {
                    if r.status != *status {
                        return false;
                    }
                }
                if let Some(since) = filter.since {
                    if r.submitted_at < since {
                        return false;
                    }
                }
                true
            })
            .take(filter.limit)
            .cloned()
            .collect()
    }

    async fn stats(&self) -> DispatcherStats {
        let now = jiff::Timestamp::now();
        let uptime = (now.as_second() - self.started_at.as_second()).unsigned_abs();
        DispatcherStats {
            total_submitted: self.submitted.load(Ordering::Relaxed),
            total_completed: self.completed.load(Ordering::Relaxed),
            total_errors: self.errors.load(Ordering::Relaxed),
            total_deduped: self.deduped.load(Ordering::Relaxed),
            total_cancelled: self.cancelled.load(Ordering::Relaxed),
            uptime_seconds: uptime,
        }
    }
}
