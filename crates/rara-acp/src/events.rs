//! Event types bridging ACP session updates to rara's internal event system.

use std::path::PathBuf;

use agent_client_protocol::RequestPermissionOutcome;
use tokio::sync::oneshot;

/// Events emitted by an ACP agent session, consumed by the rara kernel.
#[derive(Debug, Clone)]
pub enum AcpEvent {
    /// Agent is thinking / reasoning (streaming chunk).
    Thinking(String),

    /// Agent is producing text output (streaming chunk).
    Text(String),

    /// Agent initiated a tool call.
    ToolCallStarted {
        /// Unique identifier for this tool invocation.
        id:    String,
        /// Human-readable title of the tool call.
        title: String,
    },

    /// Tool call status update (running, completed, failed).
    ToolCallUpdate {
        /// Identifier matching a previous [`AcpEvent::ToolCallStarted`].
        id:     String,
        /// Current status of the tool call.
        status: ToolCallStatus,
        /// Optional textual output from the tool.
        output: Option<String>,
    },

    /// Agent produced a structured plan.
    Plan {
        /// Optional plan title.
        title: Option<String>,
        /// Individual plan steps.
        steps: Vec<String>,
    },

    /// Agent's turn ended.
    TurnComplete {
        /// The reason the turn finished.
        stop_reason: StopReason,
    },

    /// Agent process exited.
    ProcessExited {
        /// Exit code, if available.
        code: Option<i32>,
    },

    /// Agent requested a permission that was auto-approved by rara.
    PermissionAutoApproved {
        /// Human-readable description of what was approved.
        description: String,
    },

    /// Agent requests permission — needs user confirmation.
    PermissionRequested {
        /// Tool call ID for correlation with
        /// [`crate::thread::AcpThread::authorize_tool_call`].
        tool_call_id: String,
        /// Human-readable description of what the agent wants to do.
        tool_title:   String,
        /// Available options the user can choose from.
        options:      Vec<PermissionOptionInfo>,
    },

    /// Agent read or wrote a file on disk.
    FileAccess {
        /// Filesystem path that was accessed.
        path:      PathBuf,
        /// Whether the access was a read or a write.
        operation: FileOperation,
    },
}

/// Status of an agent tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolCallStatus {
    /// The tool is currently executing.
    Running,
    /// The tool finished successfully.
    Completed,
    /// The tool encountered an error.
    Failed,
}

/// Why the agent's turn ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    /// The agent decided to end its turn normally.
    EndTurn,
    /// The agent reached the maximum number of tokens.
    MaxTokens,
    /// The agent refused to continue the task.
    Refusal,
    /// The turn was cancelled by the client.
    Cancelled,
    /// The turn ended due to an error.
    Error(String),
}

/// File operation performed by the agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileOperation {
    /// The agent read a file.
    Read,
    /// The agent wrote a file.
    Write,
}

/// A permission request bridged from the `!Send` delegate to the `Send` world.
///
/// Carries the original ACP request and a oneshot channel for the user's
/// decision. The handler **must** respond via `reply_tx` — dropping it causes
/// the delegate to return `Cancelled`.
pub struct PermissionBridge {
    /// Human-readable title of the tool call requesting permission.
    pub tool_title:   String,
    /// Tool call ID for correlation.
    pub tool_call_id: String,
    /// Available permission options (simplified for display).
    pub options:      Vec<PermissionOptionInfo>,
    /// Oneshot sender to deliver the user's decision.
    pub reply_tx:     oneshot::Sender<RequestPermissionOutcome>,
}

/// Kind of permission option, mirroring the upstream ACP protocol types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionOptionKind {
    /// Allow this specific invocation only.
    AllowOnce,
    /// Allow all future invocations of this tool.
    AllowAlways,
    /// Deny this specific invocation.
    DenyOnce,
    /// Deny all future invocations.
    DenyAlways,
    /// An unknown/unrecognized option kind from the protocol.
    Unknown(String),
}

/// Simplified permission option for UI display.
#[derive(Debug, Clone)]
pub struct PermissionOptionInfo {
    /// Option identifier to send back when selected.
    pub id:    String,
    /// Human-readable label.
    pub label: String,
    /// Option kind.
    pub kind:  PermissionOptionKind,
}
