use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::sync::{RwLock, broadcast};

use crate::config::SymphonyConfig;

/// A snapshot of Symphony's runtime state, safe to share across threads.
#[derive(Debug, Clone, Serialize)]
pub struct SymphonySnapshot {
    pub running: Vec<RunInfo>,
    pub claimed: Vec<String>,
    pub retries: Vec<RetryInfo>,
    pub config_summary: ConfigSummary,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunInfo {
    pub issue_id: String,
    pub repo: String,
    pub title: String,
    pub workspace_path: String,
    pub branch: String,
    pub started_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RetryInfo {
    pub issue_id: String,
    pub attempt: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigSummary {
    pub enabled: bool,
    pub poll_interval_secs: u64,
    pub max_concurrent_agents: usize,
    pub repos: Vec<String>,
}

/// A serializable event for the SSE event stream.
#[derive(Debug, Clone, Serialize)]
pub struct SymphonyEventLog {
    pub timestamp: DateTime<Utc>,
    pub kind: String,
    pub issue_id: Option<String>,
    pub detail: String,
}

/// Handle for reading Symphony state from HTTP handlers.
#[derive(Clone)]
pub struct SymphonyStatusHandle {
    pub state: Arc<RwLock<SymphonySnapshot>>,
    pub events_tx: broadcast::Sender<SymphonyEventLog>,
}

impl SymphonyStatusHandle {
    pub fn new(config: &SymphonyConfig) -> Self {
        let (events_tx, _) = broadcast::channel(256);
        let snapshot = SymphonySnapshot {
            running: vec![],
            claimed: vec![],
            retries: vec![],
            config_summary: ConfigSummary {
                enabled: config.enabled,
                poll_interval_secs: config.poll_interval.as_secs(),
                max_concurrent_agents: config.max_concurrent_agents,
                repos: config.repos.iter().map(|r| r.name.clone()).collect(),
            },
            updated_at: Utc::now(),
        };
        Self {
            state: Arc::new(RwLock::new(snapshot)),
            events_tx,
        }
    }

    pub async fn update_snapshot(&self, snapshot: SymphonySnapshot) {
        *self.state.write().await = snapshot;
    }

    pub fn log_event(&self, log: SymphonyEventLog) {
        let _ = self.events_tx.send(log);
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<SymphonyEventLog> {
        self.events_tx.subscribe()
    }
}
