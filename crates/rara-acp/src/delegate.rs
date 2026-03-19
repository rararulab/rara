//! ACP client delegate that handles requests from the agent subprocess.
//!
//! The delegate implements the `acp::Client` trait, supporting both
//! auto-approve mode (for backward compatibility) and interactive permission
//! forwarding via [`PermissionBridge`].

use std::path::PathBuf;

use agent_client_protocol::{
    Client, ContentBlock, CreateTerminalRequest, CreateTerminalResponse, KillTerminalRequest,
    KillTerminalResponse, PermissionOptionKind as AcpPermOptionKind, ReadTextFileRequest,
    ReadTextFileResponse, ReleaseTerminalRequest, ReleaseTerminalResponse,
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse, Result,
    SelectedPermissionOutcome, SessionNotification, SessionUpdate, TerminalOutputRequest,
    TerminalOutputResponse, ToolCallStatus as AcpToolCallStatus, WaitForTerminalExitRequest,
    WaitForTerminalExitResponse, WriteTextFileRequest, WriteTextFileResponse,
};
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::events::{
    AcpEvent, FileOperation, PermissionBridge, PermissionOptionInfo, PermissionOptionKind,
    ToolCallStatus,
};

/// Permission handling mode for the delegate.
enum PermissionMode {
    /// Auto-approve all permission requests (original behaviour).
    AutoApprove,
    /// Forward permission requests via a channel for interactive resolution.
    Interactive(mpsc::Sender<PermissionBridge>),
}

/// ACP client delegate that bridges agent requests to rara's event system.
///
/// This type implements the ACP `Client` trait.  It is designed to be run on
/// a single-threaded `LocalSet` because the upstream trait is `!Send`.
///
/// Behaviour summary:
/// - **Permissions**: either auto-approve or forward via [`PermissionBridge`].
/// - **File I/O**: directly reads / writes files using `tokio::fs`.
/// - **Session notifications**: converted to [`AcpEvent`] and forwarded via the
///   provided mpsc sender.
/// - **Terminals**: not supported — returns `method_not_found`.
pub struct RaraDelegate {
    /// Channel for forwarding ACP events to the kernel.
    event_tx:        mpsc::Sender<AcpEvent>,
    /// How to handle permission requests.
    permission_mode: PermissionMode,
    /// Working directory for resolving relative paths.
    #[allow(dead_code)]
    cwd:             PathBuf,
}

impl RaraDelegate {
    /// Create a delegate that forwards permissions interactively.
    ///
    /// Permission requests are sent via `perm_tx` as [`PermissionBridge`]
    /// messages. The handler must reply via the oneshot channel; dropping it
    /// causes `Cancelled`.
    pub fn new(
        event_tx: mpsc::Sender<AcpEvent>,
        perm_tx: mpsc::Sender<PermissionBridge>,
        cwd: PathBuf,
    ) -> Self {
        Self {
            event_tx,
            permission_mode: PermissionMode::Interactive(perm_tx),
            cwd,
        }
    }

    /// Create a delegate that auto-approves all permission requests.
    ///
    /// This preserves the original behaviour for callers that do not need
    /// interactive permission handling.
    pub fn new_auto_approve(event_tx: mpsc::Sender<AcpEvent>, cwd: PathBuf) -> Self {
        Self {
            event_tx,
            permission_mode: PermissionMode::AutoApprove,
            cwd,
        }
    }

    /// Send an event with backpressure.
    async fn emit(&self, event: AcpEvent) {
        if self.event_tx.send(event).await.is_err() {
            warn!("ACP event channel closed — dropping event");
        }
    }

    /// Extract plain text from a [`ContentBlock`].
    fn text_from_content(block: &ContentBlock) -> String {
        match block {
            ContentBlock::Text(tc) => tc.text.clone(),
            _ => String::new(),
        }
    }

    /// Auto-approve a permission request by selecting the best "allow" option.
    async fn auto_approve(
        &self,
        args: &RequestPermissionRequest,
    ) -> Result<RequestPermissionResponse> {
        let selected = args
            .options
            .iter()
            .find(|o| o.kind == AcpPermOptionKind::AllowAlways)
            .or_else(|| {
                args.options
                    .iter()
                    .find(|o| o.kind == AcpPermOptionKind::AllowOnce)
            });

        let option_id = match selected {
            Some(opt) => opt.option_id.clone(),
            None => {
                if let Some(first) = args.options.first() {
                    first.option_id.clone()
                } else {
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
        self.emit(AcpEvent::PermissionAutoApproved { description })
            .await;

        debug!(option_id = %option_id, "auto-approved permission request");

        Ok(RequestPermissionResponse::new(
            RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(option_id)),
        ))
    }

    /// Forward a permission request interactively via the bridge channel.
    async fn interactive_permission(
        &self,
        perm_tx: &mpsc::Sender<PermissionBridge>,
        args: &RequestPermissionRequest,
    ) -> Result<RequestPermissionResponse> {
        let tool_call_id = args.tool_call.tool_call_id.to_string();
        let tool_title = args
            .tool_call
            .fields
            .title
            .clone()
            .unwrap_or_else(|| "unknown".into());

        let options: Vec<PermissionOptionInfo> = args
            .options
            .iter()
            .map(|o| PermissionOptionInfo {
                id:    o.option_id.to_string(),
                label: o.name.clone(),
                kind:  match o.kind {
                    AcpPermOptionKind::AllowOnce => PermissionOptionKind::AllowOnce,
                    AcpPermOptionKind::AllowAlways => PermissionOptionKind::AllowAlways,
                    AcpPermOptionKind::RejectOnce => PermissionOptionKind::DenyOnce,
                    AcpPermOptionKind::RejectAlways => PermissionOptionKind::DenyAlways,
                    other => PermissionOptionKind::Unknown(format!("{other:?}")),
                },
            })
            .collect();

        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();

        let bridge = PermissionBridge {
            tool_title,
            tool_call_id,
            options,
            reply_tx,
        };

        if perm_tx.send(bridge).await.is_err() {
            warn!("permission bridge channel closed — cancelling");
            return Ok(RequestPermissionResponse::new(
                RequestPermissionOutcome::Cancelled,
            ));
        }

        // Await the user's decision. If the reply channel is dropped, treat
        // it as a cancellation.
        match reply_rx.await {
            Ok(outcome) => Ok(RequestPermissionResponse::new(outcome)),
            Err(_) => {
                warn!("permission reply channel dropped — returning Cancelled");
                Ok(RequestPermissionResponse::new(
                    RequestPermissionOutcome::Cancelled,
                ))
            }
        }
    }
}

#[async_trait::async_trait(?Send)]
impl Client for RaraDelegate {
    async fn request_permission(
        &self,
        args: RequestPermissionRequest,
    ) -> Result<RequestPermissionResponse> {
        match &self.permission_mode {
            PermissionMode::AutoApprove => self.auto_approve(&args).await,
            PermissionMode::Interactive(perm_tx) => {
                self.interactive_permission(perm_tx, &args).await
            }
        }
    }

    async fn session_notification(&self, args: SessionNotification) -> Result<()> {
        match &args.update {
            SessionUpdate::AgentMessageChunk(chunk) => {
                let text = Self::text_from_content(&chunk.content);
                if !text.is_empty() {
                    self.emit(AcpEvent::Text(text)).await;
                }
            }
            SessionUpdate::AgentThoughtChunk(chunk) => {
                let text = Self::text_from_content(&chunk.content);
                if !text.is_empty() {
                    self.emit(AcpEvent::Thinking(text)).await;
                }
            }
            SessionUpdate::ToolCall(tc) => {
                self.emit(AcpEvent::ToolCallStarted {
                    id:    tc.tool_call_id.to_string(),
                    title: tc.title.clone(),
                })
                .await;
            }
            SessionUpdate::ToolCallUpdate(update) => {
                let status = match update.fields.status {
                    Some(AcpToolCallStatus::Failed) => ToolCallStatus::Failed,
                    Some(AcpToolCallStatus::Completed) => ToolCallStatus::Completed,
                    Some(AcpToolCallStatus::InProgress) => ToolCallStatus::Running,
                    _ => ToolCallStatus::Running,
                };
                // Prefer title (human-readable summary) over raw_output which
                // can be very large (e.g. full file contents from a read tool).
                let output = update.fields.title.clone().or_else(|| {
                    update.fields.raw_output.as_ref().map(|v| {
                        let s = v.to_string();
                        if s.len() > 1024 {
                            format!("{}… (truncated)", &s[..1024])
                        } else {
                            s
                        }
                    })
                });
                self.emit(AcpEvent::ToolCallUpdate {
                    id: update.tool_call_id.to_string(),
                    status,
                    output,
                })
                .await;
            }
            SessionUpdate::Plan(plan) => {
                let steps: Vec<String> = plan
                    .entries
                    .iter()
                    .map(|entry| entry.content.clone())
                    .collect();
                self.emit(AcpEvent::Plan { title: None, steps }).await;
            }
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
        })
        .await;

        let content = tokio::fs::read_to_string(path).await.map_err(|e| {
            agent_client_protocol::Error::internal_error()
                .data(format!("failed to read {}: {e}", path.display()))
        })?;

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
        })
        .await;

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

    async fn create_terminal(
        &self,
        _args: CreateTerminalRequest,
    ) -> Result<CreateTerminalResponse> {
        warn!("agent requested terminal creation, which is not supported");
        Err(agent_client_protocol::Error::method_not_found())
    }

    async fn terminal_output(
        &self,
        _args: TerminalOutputRequest,
    ) -> Result<TerminalOutputResponse> {
        warn!("agent requested terminal output, which is not supported");
        Err(agent_client_protocol::Error::method_not_found())
    }

    async fn release_terminal(
        &self,
        _args: ReleaseTerminalRequest,
    ) -> Result<ReleaseTerminalResponse> {
        warn!("agent requested terminal release, which is not supported");
        Err(agent_client_protocol::Error::method_not_found())
    }

    async fn wait_for_terminal_exit(
        &self,
        _args: WaitForTerminalExitRequest,
    ) -> Result<WaitForTerminalExitResponse> {
        warn!("agent requested wait for terminal exit, which is not supported");
        Err(agent_client_protocol::Error::method_not_found())
    }

    async fn kill_terminal(&self, _args: KillTerminalRequest) -> Result<KillTerminalResponse> {
        warn!("agent requested terminal kill, which is not supported");
        Err(agent_client_protocol::Error::method_not_found())
    }
}
