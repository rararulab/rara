use std::path::PathBuf;

use chrono::{DateTime, Utc};

/// Represents the lifecycle state of a tracked issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueState {
    /// Issue is actively being worked on or waiting for an agent.
    Active,
    /// Issue has reached a terminal state (closed, merged, etc.).
    Terminal,
}

/// An issue that symphony is tracking.
#[derive(Debug, Clone)]
pub struct TrackedIssue {
    /// Unique identifier (owner/repo#number).
    pub id: String,
    /// Human-readable identifier. GitHub: "42", Linear: "RAR-42".
    pub identifier: String,
    /// Repository name (owner/repo).
    pub repo: String,
    /// Issue number.
    pub number: u64,
    /// Issue title.
    pub title: String,
    /// Issue body/description.
    pub body: Option<String>,
    /// Labels attached to the issue.
    pub labels: Vec<String>,
    /// Priority (lower = higher priority).
    pub priority: u32,
    /// Current lifecycle state.
    pub state: IssueState,
    /// When the issue was created.
    pub created_at: DateTime<Utc>,
}

/// Information about a worktree workspace.
#[derive(Debug, Clone)]
pub struct WorkspaceInfo {
    /// Path to the worktree directory.
    pub path: PathBuf,
    /// Git branch name.
    pub branch: String,
    /// Whether this worktree was just created (vs. already existed).
    pub created_now: bool,
}

/// Events that flow through the symphony event loop.
#[derive(Debug, Clone)]
pub enum SymphonyEvent {
    /// A new issue matching active labels was discovered.
    IssueDiscovered { issue: TrackedIssue },

    /// An issue's state has changed.
    IssueStateChanged {
        issue_id: String,
        new_state: IssueState,
    },

    /// An agent completed its work successfully.
    AgentCompleted {
        issue_id: String,
        workspace: WorkspaceInfo,
    },

    /// An agent failed with an error.
    AgentFailed {
        issue_id: String,
        workspace: WorkspaceInfo,
        reason: String,
    },

    /// An agent appears to be stalled (no progress for stall_timeout).
    AgentStalled { issue_id: String },

    /// A worktree workspace was cleaned up.
    WorkspaceCleaned { issue_id: String, path: PathBuf },

    /// A failed issue is ready to be retried after backoff.
    RetryReady { issue_id: String },

    /// Graceful shutdown requested.
    Shutdown,
}
