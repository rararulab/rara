//! ACP connection lifecycle — spawn, handshake, prompt, teardown.
//!
//! [`AcpConnection`] owns a child agent process and a
//! `ClientSideConnection` that speaks the ACP JSON-RPC protocol over
//! stdin/stdout.  The connection is `!Send` because the upstream ACP crate
//! uses `async_trait(?Send)` and `LocalBoxFuture`.

use std::path::{Path, PathBuf};

use agent_client_protocol::{
    Agent, ClientCapabilities, ClientSideConnection, ContentBlock, FileSystemCapabilities,
    Implementation, InitializeRequest, NewSessionRequest, PromptRequest, PromptResponse,
    ProtocolVersion, SessionId, TextContent,
};
use snafu::ResultExt as _;
use tokio::{
    io::AsyncReadExt as _,
    process::{Child, Command},
    sync::mpsc,
};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tracing::{debug, info, warn};

use crate::{
    delegate::RaraDelegate,
    error::{self, AcpError},
    events::AcpEvent,
    registry::AgentCommand,
};

/// Handle to a running ACP agent subprocess.
///
/// The connection manages the full lifecycle: spawn the child process,
/// perform the ACP initialize handshake, create sessions, send prompts,
/// and clean up on drop.
///
/// # Threading
///
/// This type is `!Send`.  All methods must be called from the same
/// `tokio::task::LocalSet` where the connection was created.
pub struct AcpConnection {
    /// The underlying ACP RPC connection.
    conn:       ClientSideConnection,
    /// Child process handle — killed on drop.
    child:      Child,
    /// Active session id, set after [`Self::new_session`].
    session_id: Option<SessionId>,
    /// Working directory for the agent session.
    cwd:        PathBuf,
}

impl AcpConnection {
    /// Spawn the agent subprocess and perform the ACP initialize handshake.
    ///
    /// Returns the connection handle and an mpsc receiver for [`AcpEvent`]s
    /// emitted by the delegate (session notifications, file I/O, permissions).
    ///
    /// The caller **must** drive the returned connection on a `LocalSet`
    /// because the underlying ACP protocol is `!Send`.
    pub async fn connect(
        command: &AgentCommand,
        cwd: &Path,
    ) -> Result<(Self, mpsc::Receiver<AcpEvent>), AcpError> {
        // -- 1. Spawn child process ------------------------------------------
        let mut child = {
            let mut cmd = Command::new(&command.program);
            cmd.args(&command.args)
                .current_dir(cwd)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());

            for (key, value) in &command.env {
                cmd.env(key, value);
            }

            cmd.spawn().context(error::SpawnProcessSnafu)?
        };

        info!(
            program = %command.program,
            args = ?command.args,
            "spawned ACP agent subprocess"
        );

        // -- 2. Take pipes and wrap for futures::Async{Read,Write} -----------
        let child_stdin = child
            .stdin
            .take()
            .ok_or_else(|| AcpError::Handshake {
                message: "child stdin not captured".into(),
            })?
            .compat_write();

        let child_stdout = child
            .stdout
            .take()
            .ok_or_else(|| AcpError::Handshake {
                message: "child stdout not captured".into(),
            })?
            .compat();

        // -- 3. Create delegate + event channel ------------------------------
        let (event_tx, event_rx) = mpsc::channel::<AcpEvent>(256);
        let exit_event_tx = event_tx.clone();
        let delegate = RaraDelegate::new(event_tx, cwd.to_path_buf());

        // -- 4. Build ClientSideConnection -----------------------------------
        // The spawn closure runs futures on the current LocalSet via
        // `tokio::task::spawn_local`.
        let (connection, io_task) =
            ClientSideConnection::new(delegate, child_stdin, child_stdout, |future| {
                tokio::task::spawn_local(future);
            });

        // Drive the I/O loop in the background.
        tokio::task::spawn_local(async move {
            if let Err(e) = io_task.await {
                warn!(error = ?e, "ACP I/O task terminated with error");
            }
        });

        // -- 5. Watch for child process exit via stderr EOF -------------------
        // When the child process exits, its stderr pipe closes.  We read
        // stderr to EOF (discarding output) and then emit `ProcessExited`.
        let mut child_stderr = child.stderr.take();
        tokio::task::spawn_local(async move {
            if let Some(ref mut stderr) = child_stderr {
                let mut buf = [0u8; 1024];
                // Read until EOF — we discard stderr content; it's only used
                // as a signal that the process has exited.
                loop {
                    match stderr.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(_) => continue,
                    }
                }
            }
            let _ = exit_event_tx
                .send(AcpEvent::ProcessExited { code: None })
                .await;
            debug!("ACP child process exited (stderr EOF)");
        });

        // -- 6. Initialize handshake -----------------------------------------
        // Wrap `child` in an AcpConnection *before* the handshake so that
        // if `initialize()` fails, Drop will kill and reap the subprocess
        // instead of leaving it running in the background.
        let conn_handle = Self {
            conn: connection,
            child,
            session_id: None,
            cwd: cwd.to_path_buf(),
        };

        let init_request = InitializeRequest::new(ProtocolVersion::LATEST)
            .client_capabilities(
                ClientCapabilities::new()
                    .fs(FileSystemCapabilities::new()
                        .read_text_file(true)
                        .write_text_file(true))
                    .terminal(false),
            )
            .client_info(Implementation::new("rara", env!("CARGO_PKG_VERSION")));

        let init_result = conn_handle.conn.initialize(init_request).await;

        match init_result {
            Ok(init_response) => {
                debug!(
                    agent_info = ?init_response.agent_info,
                    protocol_version = %init_response.protocol_version,
                    "ACP handshake completed"
                );
                Ok((conn_handle, event_rx))
            }
            Err(e) => {
                // Drop conn_handle to kill and reap the child process.
                drop(conn_handle);
                Err(AcpError::Handshake {
                    message: format!("{e:?}"),
                })
            }
        }
    }

    /// Create a new session on the connected agent.
    ///
    /// Must be called before [`Self::send_prompt`].  Stores the session ID
    /// internally for subsequent prompt calls.
    pub async fn new_session(&mut self) -> Result<SessionId, AcpError> {
        let request = NewSessionRequest::new(&self.cwd);
        let response = self
            .conn
            .new_session(request)
            .await
            .map_err(|e| AcpError::Handshake {
                message: format!("session/new failed: {e:?}"),
            })?;

        let session_id = response.session_id;
        info!(session_id = %session_id, "ACP session created");
        self.session_id = Some(session_id.clone());
        Ok(session_id)
    }

    /// Send a user prompt to the agent and wait for the turn to complete.
    ///
    /// Returns the prompt response which includes the stop reason.
    /// Session notifications are delivered asynchronously via the event
    /// channel returned from [`Self::connect`].
    pub async fn send_prompt(&self, text: &str) -> Result<PromptResponse, AcpError> {
        let session_id = self
            .session_id
            .as_ref()
            .ok_or_else(|| AcpError::SessionNotFound {
                session_id: "<none>".into(),
            })?;

        let prompt = vec![ContentBlock::Text(TextContent::new(text))];
        let request = PromptRequest::new(session_id.clone(), prompt);

        let response = self
            .conn
            .prompt(request)
            .await
            .map_err(|e| AcpError::PromptFailed {
                message: format!("{e:?}"),
            })?;

        debug!(stop_reason = ?response.stop_reason, "prompt turn completed");
        Ok(response)
    }

    /// Close the current session, kill the child process, and reap it.
    ///
    /// Unlike `Drop` (which only does a best-effort sync reap), this method
    /// awaits the child's exit status to guarantee no zombie is left behind.
    /// Prefer calling this explicitly over relying on drop.
    pub async fn shutdown(&mut self) -> Result<(), AcpError> {
        self.session_id.take();
        self.kill_child();
        // Await the child so the OS reaps the process table entry.  After
        // start_kill() the child should exit quickly; we still await to be
        // certain no zombie is left.
        match self.child.wait().await {
            Ok(status) => {
                debug!(exit_status = ?status, "ACP child process reaped");
            }
            Err(e) => {
                // The child may already have been reaped by the stderr
                // watcher or a previous shutdown call — not an error.
                debug!(error = %e, "child wait failed (likely already reaped)");
            }
        }
        Ok(())
    }

    /// Return the active session ID, if any.
    pub fn session_id(&self) -> Option<&SessionId> { self.session_id.as_ref() }

    /// Return a reference to the underlying ACP connection for advanced usage.
    pub fn inner(&self) -> &ClientSideConnection { &self.conn }

    /// Forcibly kill the child process and attempt synchronous reap.
    ///
    /// Called from both `shutdown()` and `Drop`.  `start_kill()` sends
    /// SIGKILL; `try_wait()` reaps the process if it has already exited.
    /// In `Drop` we cannot `.await`, so `try_wait()` is the best we can
    /// do — `shutdown().await` is the preferred path for reliable reaping.
    fn kill_child(&mut self) {
        if let Err(e) = self.child.start_kill() {
            // `InvalidInput` means the child already exited — not an error.
            if e.kind() != std::io::ErrorKind::InvalidInput {
                warn!(error = %e, "failed to kill ACP agent child process");
            }
        }
        // Best-effort synchronous reap to avoid leaving a zombie when
        // the caller forgets to call shutdown().await.
        match self.child.try_wait() {
            Ok(Some(status)) => {
                debug!(exit_status = ?status, "ACP child reaped in kill_child");
            }
            Ok(None) => {
                // Process not yet exited after SIGKILL — rare but possible.
                // The async shutdown path or the OS will reap it eventually.
                debug!("ACP child not yet exited after SIGKILL, will be reaped later");
            }
            Err(e) => {
                debug!(error = %e, "try_wait failed (likely already reaped)");
            }
        }
    }
}

impl Drop for AcpConnection {
    fn drop(&mut self) { self.kill_child(); }
}
