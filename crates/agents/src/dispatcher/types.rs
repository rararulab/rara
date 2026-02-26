use std::cmp::Ordering;

use async_trait::async_trait;
use bon::Builder;
use serde::Serialize;
use tokio::sync::oneshot;

use crate::builtin::AgentOutput;

/// A task to be executed by the dispatcher.
#[derive(Builder)]
pub struct AgentTask {
    #[builder(default = ulid::Ulid::new().to_string())]
    pub id:          String,
    pub kind:        AgentTaskKind,
    pub priority:    Priority,
    pub session_key: String,
    pub message:     String,
    #[builder(default)]
    pub history:     Vec<rara_sessions::types::ChatMessage>,
    pub dedup_key:   Option<String>,
    #[builder(default = jiff::Timestamp::now())]
    pub created_at:  jiff::Timestamp,
}

/// Callback for persisting session messages after task execution.
#[async_trait]
pub trait SessionPersister: Send + Sync + 'static {
    /// Persist a user message and an assistant response to the given session.
    async fn persist_messages(
        &self,
        session_key: &str,
        user_text: &str,
        assistant_text: &str,
    ) -> Result<(), String>;

    /// Persist a single raw message to the given session.
    async fn persist_raw_message(
        &self,
        session_key: &str,
        message: &rara_sessions::types::ChatMessage,
    ) -> Result<(), String>;

    /// Ensure a session exists (create if needed).
    async fn ensure_session(&self, session_key: &str);
}

/// Callback for marking scheduled jobs as executed.
#[async_trait]
pub trait ScheduledJobCallback: Send + Sync + 'static {
    async fn mark_executed(&self, job_id: &str) -> Result<(), String>;
}

/// The kind of agent task.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum AgentTaskKind {
    Proactive,
    Scheduled { job_id: String },
    Pipeline,
}

impl AgentTaskKind {
    pub fn label(&self) -> &str {
        match self {
            AgentTaskKind::Proactive => "proactive",
            AgentTaskKind::Scheduled { .. } => "scheduled",
            AgentTaskKind::Pipeline => "pipeline",
        }
    }
}

impl std::fmt::Display for AgentTaskKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(self.label()) }
}

/// Task priority (higher value = dispatched sooner).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low = 0,
    Normal = 1,
    High = 2,
    Urgent = 3,
}

impl Priority {
    pub fn label(&self) -> &str {
        match self {
            Priority::Low => "low",
            Priority::Normal => "normal",
            Priority::High => "high",
            Priority::Urgent => "urgent",
        }
    }
}

impl PartialOrd for Priority {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> { Some(self.cmp(other)) }
}

impl Ord for Priority {
    fn cmp(&self, other: &Self) -> Ordering { (*self as u8).cmp(&(*other as u8)) }
}

/// Status of a task through its lifecycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Queued,
    Running,
    Completed,
    Error,
    Cancelled,
    Deduped,
}

/// Historical record of a task execution.
#[derive(Debug, Clone, Serialize)]
pub struct TaskRecord {
    pub id:           String,
    pub kind:         AgentTaskKind,
    pub session_key:  String,
    pub priority:     Priority,
    pub status:       TaskStatus,
    pub submitted_at: jiff::Timestamp,
    pub started_at:   Option<jiff::Timestamp>,
    pub finished_at:  Option<jiff::Timestamp>,
    pub duration_ms:  Option<u64>,
    pub error:        Option<String>,
    pub iterations:   Option<usize>,
    pub tool_calls:   Option<usize>,
}

/// Result of a completed task execution.
pub struct TaskResult {
    pub task_id: String,
    pub status:  TaskStatus,
    pub output:  Option<AgentOutput>,
    pub error:   Option<String>,
}

/// A task wrapped with its priority for the binary heap.
pub(crate) struct PrioritizedTask {
    pub task:      AgentTask,
    pub result_tx: oneshot::Sender<TaskResult>,
}

impl Eq for PrioritizedTask {}

impl PartialEq for PrioritizedTask {
    fn eq(&self, other: &Self) -> bool { self.task.id == other.task.id }
}

impl PartialOrd for PrioritizedTask {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> { Some(self.cmp(other)) }
}

impl Ord for PrioritizedTask {
    fn cmp(&self, other: &Self) -> Ordering {
        // Higher priority first; on tie, earlier created_at first (reverse).
        self.task
            .priority
            .cmp(&other.task.priority)
            .then_with(|| other.task.created_at.cmp(&self.task.created_at))
    }
}

/// Serializable summary of a queued task (for REST API).
#[derive(Debug, Clone, Serialize)]
pub struct QueuedTaskInfo {
    pub id:          String,
    pub kind:        AgentTaskKind,
    pub session_key: String,
    pub priority:    Priority,
    pub created_at:  jiff::Timestamp,
}

/// Serializable summary of a running task (for REST API).
#[derive(Debug, Clone, Serialize)]
pub struct RunningTaskInfo {
    pub id:          String,
    pub kind:        AgentTaskKind,
    pub session_key: String,
    pub priority:    Priority,
    pub started_at:  jiff::Timestamp,
}

/// Internal bookkeeping for a running task.
pub(crate) struct RunningTaskInner {
    pub info: RunningTaskInfo,
}

/// Command sent to the dispatcher run loop.
pub(crate) enum DispatcherCommand {
    Submit {
        task:      AgentTask,
        result_tx: oneshot::Sender<TaskResult>,
    },
    Cancel {
        task_id: String,
    },
}

/// Full status snapshot returned by the REST API.
#[derive(Debug, Clone, Serialize)]
pub struct DispatcherStatus {
    pub running: Vec<RunningTaskInfo>,
    pub queued:  Vec<QueuedTaskInfo>,
    pub stats:   super::log_store::DispatcherStats,
}
