//! ACP client delegate that handles requests from the agent subprocess.
//!
//! The delegate implements the `acp::Client` trait, auto-approving all
//! permission requests, performing direct file I/O, and forwarding session
//! notifications as [`AcpEvent`]s through an mpsc channel.

use std::path::PathBuf;

use agent_client_protocol::{
    Client, ContentBlock, CreateTerminalRequest, CreateTerminalResponse, KillTerminalRequest,
    KillTerminalResponse, PermissionOptionKind, ReadTextFileRequest, ReadTextFileResponse,
    ReleaseTerminalRequest, ReleaseTerminalResponse, RequestPermissionOutcome,
    RequestPermissionRequest, RequestPermissionResponse, Result, SelectedPermissionOutcome,
    SessionNotification, SessionUpdate, TerminalOutputRequest, TerminalOutputResponse,
    ToolCallStatus as AcpToolCallStatus, WaitForTerminalExitRequest, WaitForTerminalExitResponse,
    WriteTextFileRequest, WriteTextFileResponse,
};
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::events::{AcpEvent, FileOperation, ToolCallStatus};

/// ACP client delegate that bridges agent requests to rara's event system.
///
/// This type implements the ACP `Client` trait.  It is designed to be run on
/// a single-threaded `LocalSet` because the upstream trait is `!Send`.
///
/// Behaviour summary:
/// - **Permissions**: auto-approve by selecting the first `AllowAlways` (or
///   `AllowOnce`) option from the agent's permission request.
/// - **File I/O**: directly reads / writes files using `tokio::fs`.
/// - **Session notifications**: converted to [`AcpEvent`] and forwarded via the
///   provided mpsc sender.
/// - **Terminals**: not supported — returns `method_not_found`.
pub struct RaraDelegate {
    /// Channel for forwarding ACP events to the kernel.
    event_tx: mpsc::Sender<AcpEvent>,
    /// Working directory for resolving relative paths (currently unused but
    /// reserved for future sandboxing).
    #[allow(dead_code)]
    cwd:      PathBuf,
}

impl RaraDelegate {
    /// Create a new delegate that sends events to `event_tx`.
    ///
    /// `cwd` is the working directory of the agent session, used for context
    /// when emitting file-access events.
    pub fn new(event_tx: mpsc::Sender<AcpEvent>, cwd: PathBuf) -> Self { Self { event_tx, cwd } }

    /// Send an event, logging a warning if the receiver has been dropped.
    fn emit(&self, event: AcpEvent) {
        if self.event_tx.try_send(event).is_err() {
            warn!("ACP event channel full or closed — dropping event");
        }
    }

    /// Extract plain text from a [`ContentBlock`], returning an empty string
    /// for non-text variants.
    fn text_from_content(block: &ContentBlock) -> String {
        match block {
            ContentBlock::Text(tc) => tc.text.clone(),
            _ => String::new(),
        }
    }
}

#[async_trait::async_trait(?Send)]
impl Client for RaraDelegate {
    async fn request_permission(
        &self,
        args: RequestPermissionRequest,
    ) -> Result<RequestPermissionResponse> {
        // Pick the first "allow" option, preferring AllowAlways > AllowOnce.
        let selected = args
            .options
            .iter()
            .find(|o| o.kind == PermissionOptionKind::AllowAlways)
            .or_else(|| {
                args.options
                    .iter()
                    .find(|o| o.kind == PermissionOptionKind::AllowOnce)
            });

        let option_id = match selected {
            Some(opt) => opt.option_id.clone(),
            None => {
                // Fallback: pick the first option regardless of kind.
                if let Some(first) = args.options.first() {
                    first.option_id.clone()
                } else {
                    // No options available — should not happen per protocol,
                    // but cancel gracefully.
                    return Ok(RequestPermissionResponse::new(
                        RequestPermissionOutcome::Cancelled,
                    ));
                }
            }
        };

        let description = format!(
            "auto-approved tool call: {}",
            args.tool_call.fields.title.as_deref().unwrap_or("unknown")
        );
        self.emit(AcpEvent::PermissionAutoApproved { description });

        debug!(option_id = %option_id, "auto-approved permission request");

        Ok(RequestPermissionResponse::new(
            RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(option_id)),
        ))
    }

    async fn session_notification(&self, args: SessionNotification) -> Result<()> {
        match &args.update {
            SessionUpdate::AgentMessageChunk(chunk) => {
                let text = Self::text_from_content(&chunk.content);
                if !text.is_empty() {
                    self.emit(AcpEvent::Text(text));
                }
            }
            SessionUpdate::AgentThoughtChunk(chunk) => {
                let text = Self::text_from_content(&chunk.content);
                if !text.is_empty() {
                    self.emit(AcpEvent::Thinking(text));
                }
            }
            SessionUpdate::ToolCall(tc) => {
                self.emit(AcpEvent::ToolCallStarted {
                    id:    tc.tool_call_id.to_string(),
                    title: tc.title.clone(),
                });
            }
            SessionUpdate::ToolCallUpdate(update) => {
                let status = match update.fields.status {
                    Some(AcpToolCallStatus::Failed) => ToolCallStatus::Failed,
                    Some(AcpToolCallStatus::Completed) => ToolCallStatus::Completed,
                    Some(AcpToolCallStatus::InProgress) => ToolCallStatus::Running,
                    _ => ToolCallStatus::Running,
                };
                self.emit(AcpEvent::ToolCallUpdate {
                    id: update.tool_call_id.to_string(),
                    status,
                    output: update.fields.title.clone(),
                });
            }
            SessionUpdate::Plan(plan) => {
                let steps: Vec<String> = plan
                    .entries
                    .iter()
                    .map(|entry| entry.content.clone())
                    .collect();
                self.emit(AcpEvent::Plan { title: None, steps });
            }
            // Ignore updates we don't translate yet (mode changes, commands, etc.)
            _ => {
                debug!(update = ?args.update, "ignoring unhandled session update variant");
            }
        }
        Ok(())
    }

    async fn read_text_file(&self, args: ReadTextFileRequest) -> Result<ReadTextFileResponse> {
        let path = &args.path;

        self.emit(AcpEvent::FileAccess {
            path:      path.clone(),
            operation: FileOperation::Read,
        });

        let content = tokio::fs::read_to_string(path).await.map_err(|e| {
            agent_client_protocol::Error::internal_error()
                .data(format!("failed to read {}: {e}", path.display()))
        })?;

        // Handle optional line/limit slicing.
        let content = if args.line.is_some() || args.limit.is_some() {
            let lines: Vec<&str> = content.lines().collect();
            let start = args.line.unwrap_or(1).saturating_sub(1) as usize;
            let end = match args.limit {
                Some(limit) => (start + limit as usize).min(lines.len()),
                None => lines.len(),
            };
            if start < lines.len() {
                lines[start..end].join("\n")
            } else {
                String::new()
            }
        } else {
            content
        };

        Ok(ReadTextFileResponse::new(content))
    }

    async fn write_text_file(&self, args: WriteTextFileRequest) -> Result<WriteTextFileResponse> {
        let path = &args.path;

        self.emit(AcpEvent::FileAccess {
            path:      path.clone(),
            operation: FileOperation::Write,
        });

        // Ensure parent directory exists.
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                agent_client_protocol::Error::internal_error().data(format!(
                    "failed to create parent dir for {}: {e}",
                    path.display()
                ))
            })?;
        }

        tokio::fs::write(path, &args.content).await.map_err(|e| {
            agent_client_protocol::Error::internal_error()
                .data(format!("failed to write {}: {e}", path.display()))
        })?;

        Ok(WriteTextFileResponse::new())
    }

    // Terminal methods are not supported — the default implementations in the
    // trait already return `Error::method_not_found()`.  We explicitly list
    // them here for clarity and to prevent accidental future breakage if the
    // upstream trait removes default impls.

    async fn create_terminal(
        &self,
        _args: CreateTerminalRequest,
    ) -> Result<CreateTerminalResponse> {
        Err(agent_client_protocol::Error::method_not_found())
    }

    async fn terminal_output(
        &self,
        _args: TerminalOutputRequest,
    ) -> Result<TerminalOutputResponse> {
        Err(agent_client_protocol::Error::method_not_found())
    }

    async fn release_terminal(
        &self,
        _args: ReleaseTerminalRequest,
    ) -> Result<ReleaseTerminalResponse> {
        Err(agent_client_protocol::Error::method_not_found())
    }

    async fn wait_for_terminal_exit(
        &self,
        _args: WaitForTerminalExitRequest,
    ) -> Result<WaitForTerminalExitResponse> {
        Err(agent_client_protocol::Error::method_not_found())
    }

    async fn kill_terminal(&self, _args: KillTerminalRequest) -> Result<KillTerminalResponse> {
        Err(agent_client_protocol::Error::method_not_found())
    }
}
