//! AcpThread — persistent, interactive session with an external code agent.
//!
//! [`AcpThread`] is a `Send + Sync` handle that manages the lifecycle of a
//! conversation with an external ACP agent. It decouples the external agent's
//! session from rara's own Session, supporting multi-turn prompts, interactive
//! permission requests, and streaming event forwarding.

use std::{collections::HashMap, path::PathBuf};

use agent_client_protocol::RequestPermissionOutcome;
use futures::future::BoxFuture;
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};
use tracing::{debug, info, warn};

use crate::{
    error::AcpError,
    events::{AcpEvent, PermissionBridge, PermissionOptionInfo, StopReason, ToolCallStatus},
    registry::{AgentKind, AgentRegistry},
};

/// Commands sent from AcpThread (Send) to the connection actor (!Send).
pub(crate) enum AcpCommand {
    /// Send a user prompt to the agent.
    Prompt {
        text:     String,
        reply_tx: oneshot::Sender<Result<StopReason, AcpError>>,
    },
    /// Gracefully shut down the connection.
    Shutdown { reply_tx: oneshot::Sender<()> },
}

/// Status of an AcpThread.
#[derive(Debug, Clone)]
pub enum AcpThreadStatus {
    /// Connection established, waiting for prompts.
    Ready,
    /// A prompt is running; the agent is generating.
    Generating,
    /// Blocked: waiting for user to approve/deny a permission request.
    WaitingForConfirmation {
        /// Tool call that triggered the request.
        tool_call_id: String,
        /// Human-readable description.
        tool_title:   String,
        /// Available options.
        options:      Vec<PermissionOptionInfo>,
    },
    /// The agent's turn ended; ready for follow-up or shutdown.
    TurnComplete {
        /// Why the turn ended.
        stop_reason: StopReason,
    },
    /// The agent process has exited.
    Disconnected,
}

/// A conversation entry in the AcpThread.
#[derive(Debug, Clone)]
pub enum AcpThreadEntry {
    /// User prompt sent to the agent.
    UserMessage(String),
    /// Agent text output (accumulated from streaming chunks).
    AssistantMessage(String),
    /// Agent thought/reasoning text.
    Thinking(String),
    /// A tool call lifecycle event.
    ToolCall {
        /// Tool call identifier.
        id:     String,
        /// Human-readable title.
        title:  String,
        /// Current status as string.
        status: String,
        /// Optional output from the tool.
        output: Option<String>,
    },
    /// A structured plan from the agent.
    Plan {
        /// Optional plan title.
        title: Option<String>,
        /// Individual plan steps.
        steps: Vec<String>,
    },
}

/// A tool call tracked by the AcpThread.
pub struct AcpToolCall {
    /// Tool call identifier.
    pub id:     String,
    /// Human-readable title.
    pub title:  String,
    /// Current lifecycle status.
    pub status: AcpToolCallStatus,
    /// Output from the tool, if available.
    pub output: Option<String>,
}

/// Status of a tool call within the AcpThread.
pub enum AcpToolCallStatus {
    /// Shown to the user but not yet running.
    Pending,
    /// Waiting for user to approve/deny.
    WaitingForConfirmation {
        /// Available options.
        options:  Vec<PermissionOptionInfo>,
        /// Oneshot sender for the user's decision back to the ACP delegate.
        reply_tx: oneshot::Sender<RequestPermissionOutcome>,
    },
    /// Currently executing.
    Running,
    /// Finished successfully.
    Completed,
    /// Encountered an error.
    Failed,
    /// User rejected the permission request.
    Rejected,
}

/// Information about a permission request, passed to the resolver callback.
///
/// This is the `Send`-safe subset of [`PermissionBridge`] — the `reply_tx`
/// oneshot is stored internally in the [`AcpToolCall`] and sent automatically
/// when the resolver returns.
#[derive(Debug, Clone)]
pub struct PermissionRequestInfo {
    /// Tool call ID for correlation.
    pub tool_call_id: String,
    /// Human-readable title of the tool call requesting permission.
    pub tool_title:   String,
    /// Available permission options (simplified for display).
    pub options:      Vec<PermissionOptionInfo>,
}

/// A `Send + Sync` handle to an ACP agent conversation.
///
/// Manages the full lifecycle of an external code agent session, decoupled
/// from rara's own Session. One rara Session may spawn multiple AcpThreads
/// (e.g., delegate to Claude, then Codex).
pub struct AcpThread {
    agent_kind:    AgentKind,
    session_id:    Option<agent_client_protocol::SessionId>,
    status:        AcpThreadStatus,
    entries:       Vec<AcpThreadEntry>,
    tool_calls:    HashMap<String, AcpToolCall>,
    command_tx:    mpsc::Sender<AcpCommand>,
    event_rx:      mpsc::Receiver<AcpEvent>,
    permission_rx: mpsc::Receiver<PermissionBridge>,
    actor_handle:  Option<JoinHandle<()>>,
}

impl AcpThread {
    /// Spawn a new AcpThread: starts the agent subprocess, performs handshake,
    /// creates ACP session. Returns a ready-to-use handle.
    pub async fn spawn(agent_kind: AgentKind, cwd: PathBuf) -> Result<Self, AcpError> {
        let registry = AgentRegistry::with_defaults();
        let command = registry
            .resolve(&agent_kind)
            .ok_or_else(|| AcpError::Handshake {
                message: format!("unknown agent kind: {agent_kind:?}"),
            })?
            .clone();

        // Channels: command (thread -> actor), event (actor -> thread),
        // permission (delegate -> thread).
        let (cmd_tx, cmd_rx) = mpsc::channel::<AcpCommand>(8);
        let (event_tx, event_rx) = mpsc::channel::<AcpEvent>(256);
        let (perm_tx, perm_rx) = mpsc::channel::<PermissionBridge>(8);

        // Session ID delivered after handshake via a oneshot.
        let (session_id_tx, session_id_rx) = oneshot::channel();

        let cwd_clone = cwd.clone();
        let actor_handle = tokio::task::spawn_blocking(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build current-thread runtime for ACP");

            let local = tokio::task::LocalSet::new();
            local.block_on(&rt, async move {
                run_connection_actor(command, cwd_clone, cmd_rx, event_tx, perm_tx, session_id_tx)
                    .await;
            });
        });

        // Wait for the actor to complete handshake and report session ID.
        let session_id = session_id_rx.await.map_err(|_| AcpError::Handshake {
            message: "connection actor exited before reporting session ID".into(),
        })??;

        info!(
            agent = ?agent_kind,
            session_id = %session_id,
            "AcpThread spawned"
        );

        Ok(Self {
            agent_kind,
            session_id: Some(session_id),
            status: AcpThreadStatus::Ready,
            entries: Vec::new(),
            tool_calls: HashMap::new(),
            command_tx: cmd_tx,
            event_rx,
            permission_rx: perm_rx,
            actor_handle: Some(actor_handle),
        })
    }

    /// Send a prompt and drive the session until the turn completes.
    ///
    /// Processes events internally (accumulating entries, handling permissions)
    /// and calls `on_event` for each event so the caller can forward streaming
    /// updates to the user.
    ///
    /// For permission requests, `resolve_permission` is called with a
    /// [`PermissionRequestInfo`] (the `reply_tx` is stored internally). The
    /// returned future must resolve to the user's decision
    /// ([`RequestPermissionOutcome`]). The prompt loop sends the outcome back
    /// to the ACP delegate automatically.
    pub async fn prompt(
        &mut self,
        text: &str,
        mut on_event: impl FnMut(&AcpEvent),
        resolve_permission: impl Fn(
            PermissionRequestInfo,
        ) -> BoxFuture<'static, RequestPermissionOutcome>,
    ) -> Result<StopReason, AcpError> {
        self.entries
            .push(AcpThreadEntry::UserMessage(text.to_string()));
        self.status = AcpThreadStatus::Generating;

        let (reply_tx, mut reply_rx) = oneshot::channel();
        self.command_tx
            .send(AcpCommand::Prompt {
                text: text.to_string(),
                reply_tx,
            })
            .await
            .map_err(|_| AcpError::Handshake {
                message: "command channel closed".into(),
            })?;

        let mut assistant_text = String::new();

        // Active permission resolution futures.
        let mut pending_permissions: Vec<(
            String, // tool_call_id
            tokio::task::JoinHandle<(String, RequestPermissionOutcome)>,
        )> = Vec::new();

        loop {
            tokio::select! {
                // Event from the connection actor.
                event = self.event_rx.recv() => {
                    let Some(event) = event else {
                        self.status = AcpThreadStatus::Disconnected;
                        return Err(AcpError::Handshake {
                            message: "event channel closed unexpectedly".into(),
                        });
                    };

                    match &event {
                        AcpEvent::Text(text) => {
                            assistant_text.push_str(text);
                        }
                        AcpEvent::Thinking(text) => {
                            self.entries.push(AcpThreadEntry::Thinking(text.clone()));
                        }
                        AcpEvent::ToolCallStarted { id, title } => {
                            self.tool_calls.insert(id.clone(), AcpToolCall {
                                id: id.clone(),
                                title: title.clone(),
                                status: AcpToolCallStatus::Running,
                                output: None,
                            });
                            self.entries.push(AcpThreadEntry::ToolCall {
                                id: id.clone(),
                                title: title.clone(),
                                status: "started".into(),
                                output: None,
                            });
                        }
                        AcpEvent::ToolCallUpdate { id, status, output } => {
                            if let Some(tc) = self.tool_calls.get_mut(id) {
                                tc.status = match status {
                                    ToolCallStatus::Running => AcpToolCallStatus::Running,
                                    ToolCallStatus::Completed => AcpToolCallStatus::Completed,
                                    ToolCallStatus::Failed => AcpToolCallStatus::Failed,
                                };
                                tc.output.clone_from(output);
                            }
                        }
                        AcpEvent::Plan { title, steps } => {
                            self.entries.push(AcpThreadEntry::Plan {
                                title: title.clone(),
                                steps: steps.clone(),
                            });
                        }
                        AcpEvent::ProcessExited { .. } => {
                            self.status = AcpThreadStatus::Disconnected;
                        }
                        _ => {}
                    }

                    on_event(&event);
                }

                // Permission request from the delegate.
                bridge = self.permission_rx.recv() => {
                    let Some(bridge) = bridge else { continue };

                    let tool_call_id = bridge.tool_call_id.clone();
                    let tool_title = bridge.tool_title.clone();
                    let options = bridge.options.clone();

                    // Store the reply_tx in the tool call so
                    // resolve_permission_outcome() can send the decision back.
                    if let Some(tc) = self.tool_calls.get_mut(&tool_call_id) {
                        tc.status = AcpToolCallStatus::WaitingForConfirmation {
                            options: options.clone(),
                            reply_tx: bridge.reply_tx,
                        };
                    } else {
                        // Tool call not yet registered — create it.
                        self.tool_calls.insert(tool_call_id.clone(), AcpToolCall {
                            id: tool_call_id.clone(),
                            title: tool_title.clone(),
                            status: AcpToolCallStatus::WaitingForConfirmation {
                                options: options.clone(),
                                reply_tx: bridge.reply_tx,
                            },
                            output: None,
                        });
                    }

                    self.status = AcpThreadStatus::WaitingForConfirmation {
                        tool_call_id: tool_call_id.clone(),
                        tool_title: tool_title.clone(),
                        options: options.clone(),
                    };

                    // Notify the caller.
                    let perm_event = AcpEvent::PermissionRequested {
                        tool_call_id: tool_call_id.clone(),
                        tool_title: tool_title.clone(),
                        options: options.clone(),
                    };
                    on_event(&perm_event);

                    // Build the info struct (without reply_tx) for the resolver.
                    let info = PermissionRequestInfo {
                        tool_call_id: tool_call_id.clone(),
                        tool_title,
                        options,
                    };

                    // Spawn the resolver so we can continue processing events
                    // while waiting for the user's decision.
                    let tc_id = tool_call_id.clone();
                    let future = resolve_permission(info);
                    let handle = tokio::spawn(async move {
                        let outcome = future.await;
                        (tc_id, outcome)
                    });
                    pending_permissions.push((tool_call_id, handle));
                }

                // Prompt completed.
                result = &mut reply_rx => {
                    // Flush remaining assistant text.
                    if !assistant_text.is_empty() {
                        self.entries.push(AcpThreadEntry::AssistantMessage(
                            std::mem::take(&mut assistant_text),
                        ));
                    }

                    let stop_reason = result
                        .map_err(|_| AcpError::Handshake {
                            message: "prompt reply channel closed".into(),
                        })??;

                    self.status = AcpThreadStatus::TurnComplete {
                        stop_reason: stop_reason.clone(),
                    };
                    return Ok(stop_reason);
                }
            }

            // Check if any permission resolutions have completed.
            let mut i = 0;
            while i < pending_permissions.len() {
                if pending_permissions[i].1.is_finished() {
                    let (_, handle) = pending_permissions.swap_remove(i);
                    if let Ok((tc_id, outcome)) = handle.await {
                        self.resolve_permission_outcome(&tc_id, outcome);
                    }
                } else {
                    i += 1;
                }
            }
        }
    }

    /// Send the user's permission decision back to the ACP delegate via the
    /// stored oneshot channel.
    fn resolve_permission_outcome(
        &mut self,
        tool_call_id: &str,
        outcome: RequestPermissionOutcome,
    ) {
        let Some(tc) = self.tool_calls.get_mut(tool_call_id) else {
            warn!(tool_call_id, "permission resolved for unknown tool call");
            return;
        };

        let prev = std::mem::replace(&mut tc.status, AcpToolCallStatus::Running);

        if let AcpToolCallStatus::WaitingForConfirmation { reply_tx, .. } = prev {
            let is_rejection = matches!(&outcome, RequestPermissionOutcome::Cancelled);
            if is_rejection {
                tc.status = AcpToolCallStatus::Rejected;
            }
            if reply_tx.send(outcome).is_err() {
                warn!(tool_call_id, "permission reply channel already closed");
            }
            self.status = AcpThreadStatus::Generating;
        } else {
            // Restore the previous status since we didn't consume a reply_tx.
            tc.status = prev;
            warn!(
                tool_call_id,
                "tool call not in WaitingForConfirmation state — ignoring outcome"
            );
        }
    }

    /// Resolve a pending permission request externally.
    ///
    /// Called after the user approves/denies in the UI. Sends the outcome back
    /// to the delegate via the stored oneshot channel.
    ///
    /// Returns an error if the tool call does not exist or is not waiting for
    /// confirmation.
    pub fn authorize_tool_call(
        &mut self,
        tool_call_id: &str,
        outcome: RequestPermissionOutcome,
    ) -> Result<(), AcpError> {
        let tc =
            self.tool_calls
                .get_mut(tool_call_id)
                .ok_or_else(|| AcpError::SessionNotFound {
                    session_id: tool_call_id.into(),
                })?;

        let prev = std::mem::replace(&mut tc.status, AcpToolCallStatus::Running);

        if let AcpToolCallStatus::WaitingForConfirmation { reply_tx, .. } = prev {
            let is_rejection = matches!(&outcome, RequestPermissionOutcome::Cancelled);
            if is_rejection {
                tc.status = AcpToolCallStatus::Rejected;
            }
            if reply_tx.send(outcome).is_err() {
                warn!(tool_call_id, "permission reply channel already closed");
            }
            self.status = AcpThreadStatus::Generating;
            Ok(())
        } else {
            // Restore previous status.
            tc.status = prev;
            Err(AcpError::Protocol {
                message: format!("tool call '{tool_call_id}' is not waiting for confirmation"),
            })
        }
    }

    /// Gracefully shut down: close session, kill subprocess, reap.
    pub async fn shutdown(mut self) -> Result<(), AcpError> {
        let (tx, rx) = oneshot::channel();
        let _ = self
            .command_tx
            .send(AcpCommand::Shutdown { reply_tx: tx })
            .await;
        let _ = rx.await;

        if let Some(handle) = self.actor_handle.take() {
            let _ = handle.await;
        }

        self.status = AcpThreadStatus::Disconnected;
        debug!("AcpThread shut down");
        Ok(())
    }

    /// Current thread status.
    pub fn status(&self) -> &AcpThreadStatus { &self.status }

    /// All conversation entries.
    pub fn entries(&self) -> &[AcpThreadEntry] { &self.entries }

    /// The ACP session ID.
    pub fn session_id(&self) -> Option<&agent_client_protocol::SessionId> {
        self.session_id.as_ref()
    }

    /// The agent kind.
    pub fn agent_kind(&self) -> &AgentKind { &self.agent_kind }
}

/// Connection actor: runs on a dedicated single-threaded runtime + LocalSet.
///
/// Owns the `!Send` AcpConnection and processes commands from the AcpThread
/// handle.
async fn run_connection_actor(
    command: crate::registry::AgentCommand,
    cwd: PathBuf,
    mut cmd_rx: mpsc::Receiver<AcpCommand>,
    event_fwd_tx: mpsc::Sender<AcpEvent>,
    perm_tx: mpsc::Sender<PermissionBridge>,
    session_id_tx: oneshot::Sender<Result<agent_client_protocol::SessionId, AcpError>>,
) {
    // Connect and handshake.
    let (mut conn, mut event_rx) =
        match crate::AcpConnection::connect(&command, &cwd, Some(perm_tx)).await {
            Ok(pair) => pair,
            Err(e) => {
                let _ = session_id_tx.send(Err(e));
                return;
            }
        };

    // Create session.
    let session_id = match conn.new_session().await {
        Ok(id) => id,
        Err(e) => {
            let _ = session_id_tx.send(Err(e));
            return;
        }
    };

    // Report success.
    let _ = session_id_tx.send(Ok(session_id));

    // Forward events from delegate to the AcpThread handle.
    let fwd_tx = event_fwd_tx.clone();
    tokio::task::spawn_local(async move {
        while let Some(event) = event_rx.recv().await {
            if fwd_tx.send(event).await.is_err() {
                break;
            }
        }
    });

    // Process commands from the AcpThread handle.
    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            AcpCommand::Prompt { text, reply_tx } => {
                let result = conn.send_prompt(&text).await;
                let mapped = result.map(|resp| match resp.stop_reason {
                    agent_client_protocol::StopReason::EndTurn => StopReason::EndTurn,
                    agent_client_protocol::StopReason::Cancelled => StopReason::Cancelled,
                    _ => StopReason::EndTurn,
                });
                let _ = reply_tx.send(mapped);
            }
            AcpCommand::Shutdown { reply_tx } => {
                let _ = conn.shutdown().await;
                let _ = reply_tx.send(());
                break;
            }
        }
    }
}

impl Drop for AcpThread {
    fn drop(&mut self) {
        // Best-effort: tell the actor to shut down.
        // Can't await here, but the actor will clean up the child process
        // via AcpConnection::Drop.
        let _ = self.command_tx.try_send(AcpCommand::Shutdown {
            reply_tx: oneshot::channel().0,
        });
    }
}
