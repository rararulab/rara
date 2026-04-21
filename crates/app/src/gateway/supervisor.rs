// Copyright 2025 Rararulab
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

//! [`SupervisorService`] — spawn, health-check, and restart `rara server`.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use serde::Serialize;
use snafu::{ResultExt, Snafu};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
    sync::{mpsc, watch},
};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use super::notifier::UpdateNotifier;
use crate::GatewayConfig;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Snafu)]
pub enum SupervisorError {
    #[snafu(display("Failed to spawn child process: {source}"))]
    Spawn { source: std::io::Error },

    #[snafu(display("Child process did not emit READY within {timeout_secs}s"))]
    ReadyTimeout { timeout_secs: u64 },

    #[snafu(display("HTTP health check did not pass within {timeout_secs}s"))]
    HealthTimeout { timeout_secs: u64 },

    #[snafu(display("Failed to send signal to child: {source}"))]
    Signal { source: std::io::Error },

    #[snafu(display("Max restart attempts ({max}) exhausted"))]
    MaxRestartsExhausted { max: u32 },
}

// ---------------------------------------------------------------------------
// Command & Handle
// ---------------------------------------------------------------------------

/// Commands that can be sent to the supervisor via [`SupervisorHandle`].
#[derive(Debug)]
pub enum SupervisorCommand {
    /// Gracefully restart the agent process (does not count as a failure).
    Restart,
    /// Gracefully shut down the agent and exit the supervisor loop.
    Shutdown,
}

/// Live status snapshot exposed by the supervisor.
#[derive(Debug, Clone, Serialize)]
pub struct SupervisorStatus {
    /// Whether the agent child process is currently running.
    pub running:       bool,
    /// Number of times the agent has been restarted (failure + manual).
    pub restart_count: u32,
    /// PID of the running child process, if any.
    pub pid:           Option<u32>,
}

/// A clonable handle for sending commands to a running [`SupervisorService`]
/// and reading its status.
#[derive(Clone)]
pub struct SupervisorHandle {
    cmd_tx:    mpsc::Sender<SupervisorCommand>,
    status_rx: watch::Receiver<SupervisorStatus>,
}

impl SupervisorHandle {
    /// Request the supervisor to restart the agent process.
    pub async fn restart(&self) -> Result<(), mpsc::error::SendError<SupervisorCommand>> {
        self.cmd_tx.send(SupervisorCommand::Restart).await
    }

    /// Request the supervisor to shut down.
    pub async fn shutdown(&self) -> Result<(), mpsc::error::SendError<SupervisorCommand>> {
        self.cmd_tx.send(SupervisorCommand::Shutdown).await
    }

    /// Get a snapshot of the current supervisor status.
    pub fn status(&self) -> SupervisorStatus { self.status_rx.borrow().clone() }
}

// ---------------------------------------------------------------------------
// SupervisorService
// ---------------------------------------------------------------------------

/// Manages a single `rara server` child process with health checks and
/// restart-with-backoff semantics.
pub struct SupervisorService {
    config:        GatewayConfig,
    health_url:    String,
    child:         Option<Child>,
    restart_count: u32,
    last_healthy:  Option<Instant>,
    shutdown:      CancellationToken,
    cmd_rx:        mpsc::Receiver<SupervisorCommand>,
    status_tx:     watch::Sender<SupervisorStatus>,
    notifier:      Arc<UpdateNotifier>,
}

impl SupervisorService {
    /// Create a new supervisor and its associated [`SupervisorHandle`].
    ///
    /// `health_port` is the HTTP port the agent binds to (from
    /// `RestServerConfig::bind_address`).
    pub fn new(
        config: GatewayConfig,
        health_port: &str,
        notifier: Arc<UpdateNotifier>,
    ) -> (Self, SupervisorHandle) {
        let health_url = format!("http://127.0.0.1:{health_port}/api/health");
        let shutdown = CancellationToken::new();

        // Spawn a task that cancels the token on SIGTERM/SIGINT.
        let token = shutdown.clone();
        tokio::spawn(async move {
            Self::wait_for_signal().await;
            token.cancel();
        });

        let (cmd_tx, cmd_rx) = mpsc::channel(8);

        let initial_status = SupervisorStatus {
            running:       false,
            restart_count: 0,
            pid:           None,
        };
        let (status_tx, status_rx) = watch::channel(initial_status);

        let service = Self {
            config,
            health_url,
            child: None,
            restart_count: 0,
            last_healthy: None,
            shutdown,
            cmd_rx,
            status_tx,
            notifier,
        };

        let handle = SupervisorHandle { cmd_tx, status_rx };

        (service, handle)
    }

    /// Publish a status update to watchers.
    fn publish_status(&self) {
        let pid = self.child.as_ref().and_then(|c| c.id());
        let _ = self.status_tx.send(SupervisorStatus {
            running: self.child.is_some(),
            restart_count: self.restart_count,
            pid,
        });
    }

    /// Run the supervisor loop until a shutdown signal is received or
    /// max restarts are exhausted.
    ///
    /// This is the main entry point — it spawns the child, monitors it,
    /// and restarts on failure with exponential backoff.
    pub async fn run(&mut self) -> Result<(), SupervisorError> {
        loop {
            if self.shutdown.is_cancelled() {
                info!("Shutdown requested — exiting supervisor loop");
                return Ok(());
            }

            // Reset restart counter if healthy for long enough.
            if let Some(ts) = self.last_healthy {
                if ts.elapsed() >= Duration::from_mins(1) {
                    if self.restart_count > 0 {
                        info!("Agent healthy for 60s — resetting restart counter");
                    }
                    self.restart_count = 0;
                }
            }

            info!(attempt = self.restart_count + 1, "Spawning agent process");

            match self.spawn_and_monitor().await {
                Ok(ExitReason::ShutdownRequested) => {
                    self.publish_status();
                    return Ok(());
                }
                Ok(ExitReason::ManualRestart) => {
                    info!("Manual restart requested — restarting agent immediately");
                    // Do not increment restart_count for manual restarts.
                    self.publish_status();
                    continue;
                }
                Ok(ExitReason::CleanExit) => {
                    // Child exited cleanly (status 0). This is unexpected for
                    // a long-running server but not an error — restart it.
                    info!("Agent process exited cleanly, restarting");
                    self.publish_status();
                }
                Err(e) => {
                    warn!(error = %e, "Agent process failed");
                    self.restart_count += 1;
                    self.publish_status();

                    if self.restart_count >= self.config.max_restart_attempts {
                        error!(
                            max = self.config.max_restart_attempts,
                            "Max restart attempts exhausted — stopping supervisor loop"
                        );
                        return Err(SupervisorError::MaxRestartsExhausted {
                            max: self.config.max_restart_attempts,
                        });
                    }

                    let backoff = Duration::from_secs(2u64.pow(self.restart_count));
                    info!(
                        backoff_secs = backoff.as_secs(),
                        "Backing off before restart"
                    );

                    tokio::select! {
                        () = tokio::time::sleep(backoff) => {}
                        () = self.shutdown.cancelled() => {
                            info!("Shutdown signal received during backoff");
                            return Ok(());
                        }
                        Some(cmd) = self.cmd_rx.recv() => {
                            match cmd {
                                SupervisorCommand::Restart => {
                                    info!("Manual restart during backoff — restarting immediately");
                                    continue;
                                }
                                SupervisorCommand::Shutdown => {
                                    info!("Shutdown command during backoff");
                                    return Ok(());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Spawn the child, run health checks, then wait for it to exit or
    /// a shutdown signal.
    async fn spawn_and_monitor(&mut self) -> Result<ExitReason, SupervisorError> {
        let mut child = self.spawn_child().await?;

        if let Some(stderr) = child.stderr.take() {
            Self::drain_child_stderr(stderr);
        }

        // Phase 1: wait for READY on stdout.
        let stdout = child.stdout.take();
        self.wait_for_ready(stdout).await?;

        // Phase 2: HTTP health polling.
        self.poll_health().await?;

        info!("Agent is healthy");
        self.last_healthy = Some(Instant::now());
        self.child = Some(child);
        self.publish_status();

        self.notifier.agent_healthy().await;

        // Wait for the child to exit, a shutdown signal, or a command.
        tokio::select! {
            status = self.child.as_mut().unwrap().wait() => {
                match status {
                    Ok(s) if s.success() => {
                        info!("Agent process exited with status 0");
                        self.child = None;
                        Ok(ExitReason::CleanExit)
                    }
                    Ok(s) => {
                        self.child = None;
                        warn!(status = %s, "Agent process exited with non-zero status");
                        Err(SupervisorError::Spawn {
                            source: std::io::Error::other(
                                format!("child exited with {s}"),
                            ),
                        })
                    }
                    Err(e) => {
                        self.child = None;
                        Err(SupervisorError::Spawn { source: e })
                    }
                }
            }
            () = self.shutdown.cancelled() => {
                info!("Shutdown signal received — stopping agent");
                self.graceful_shutdown().await;
                Ok(ExitReason::ShutdownRequested)
            }
            Some(cmd) = self.cmd_rx.recv() => {
                match cmd {
                    SupervisorCommand::Restart => {
                        info!("Restart command received — restarting agent");
                        self.graceful_shutdown().await;
                        Ok(ExitReason::ManualRestart)
                    }
                    SupervisorCommand::Shutdown => {
                        info!("Shutdown command received — stopping agent");
                        self.graceful_shutdown().await;
                        Ok(ExitReason::ShutdownRequested)
                    }
                }
            }
        }
    }

    /// Spawn `rara server` as a child process.
    ///
    /// The child is placed in its own process group (`process_group(0)`)
    /// so that Ctrl+C (SIGINT) is only delivered to the gateway, not to
    /// the child. The gateway then decides how to shut the child down.
    async fn spawn_child(&self) -> Result<Child, SupervisorError> {
        let exe = std::env::current_exe().context(SpawnSnafu)?;
        Command::new(&exe)
            .arg("server")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .process_group(0)
            .spawn()
            .context(SpawnSnafu)
    }

    /// Drain child stderr in the gateway process so the agent never writes to
    /// the controlling terminal from a background process group.
    fn drain_child_stderr(stderr: tokio::process::ChildStderr) {
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                warn!(target: "rara_app::gateway::agent_stderr", "{line}");
            }
        });
    }

    /// Phase 1: read child stdout lines until one contains "READY".
    async fn wait_for_ready(
        &self,
        stdout: Option<tokio::process::ChildStdout>,
    ) -> Result<(), SupervisorError> {
        let stdout = match stdout {
            Some(s) => s,
            None => {
                return Err(SupervisorError::ReadyTimeout {
                    timeout_secs: self.config.health_timeout / 2,
                });
            }
        };

        let timeout = Duration::from_secs(self.config.health_timeout / 2);
        let mut reader = BufReader::new(stdout).lines();

        let read_ready = async {
            while let Ok(Some(line)) = reader.next_line().await {
                if line.contains("READY") {
                    return true;
                }
            }
            false
        };

        tokio::select! {
            result = tokio::time::timeout(timeout, read_ready) => {
                match result {
                    Ok(true) => {
                        info!("Received READY from agent");
                        Ok(())
                    }
                    _ => Err(SupervisorError::ReadyTimeout {
                        timeout_secs: self.config.health_timeout / 2,
                    }),
                }
            }
            () = self.shutdown.cancelled() => {
                info!("Shutdown during READY wait");
                Err(SupervisorError::ReadyTimeout {
                    timeout_secs: 0,
                })
            }
        }
    }

    /// Phase 2: HTTP GET `/api/health` — 3 consecutive 200s required.
    async fn poll_health(&self) -> Result<(), SupervisorError> {
        let timeout = Duration::from_secs(self.config.health_timeout / 2);
        let poll_interval = self.config.health_poll_interval;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_default();

        let mut consecutive_ok = 0u32;
        let deadline = Instant::now() + timeout;

        while Instant::now() < deadline {
            if self.shutdown.is_cancelled() {
                info!("Shutdown during health polling");
                return Err(SupervisorError::HealthTimeout { timeout_secs: 0 });
            }

            match client.get(&self.health_url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    consecutive_ok += 1;
                    if consecutive_ok >= 3 {
                        return Ok(());
                    }
                }
                Ok(resp) => {
                    warn!(status = %resp.status(), "Health check returned non-200");
                    consecutive_ok = 0;
                }
                Err(e) => {
                    warn!(error = %e, "Health check request failed");
                    consecutive_ok = 0;
                }
            }

            tokio::select! {
                () = tokio::time::sleep(poll_interval) => {}
                () = self.shutdown.cancelled() => {
                    info!("Shutdown during health poll sleep");
                    return Err(SupervisorError::HealthTimeout { timeout_secs: 0 });
                }
            }
        }

        Err(SupervisorError::HealthTimeout {
            timeout_secs: self.config.health_timeout / 2,
        })
    }

    /// Graceful shutdown: SIGTERM → wait 5s → SIGKILL.
    ///
    /// Uses process-group signals because the child is spawned with
    /// `process_group(0)` (PGID = child PID).
    async fn graceful_shutdown(&mut self) {
        let Some(ref mut child) = self.child else {
            return;
        };

        let pid = match child.id() {
            Some(pid) => pid,
            None => {
                warn!("Child has no PID — already exited?");
                self.child = None;
                return;
            }
        };

        info!(pid, "Sending SIGTERM to agent process group");
        let _ = base::process_group::terminate_process_group(pid);

        // Wait up to 5 seconds for clean exit.
        match tokio::time::timeout(Duration::from_secs(5), child.wait()).await {
            Ok(Ok(status)) => {
                info!(%status, "Agent exited after SIGTERM");
            }
            _ => {
                warn!(pid, "Agent did not exit after 5s — sending SIGKILL");
                let _ = base::process_group::kill_process_group(pid);
                let _ = child.kill().await;
            }
        }

        self.child = None;
    }

    /// Wait for SIGTERM or SIGINT.
    async fn wait_for_signal() {
        let ctrl_c = tokio::signal::ctrl_c();

        #[cfg(unix)]
        {
            let mut sigterm =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("failed to install SIGTERM handler");
            tokio::select! {
                _ = ctrl_c => {}
                _ = sigterm.recv() => {}
            }
        }

        #[cfg(not(unix))]
        {
            ctrl_c.await.ok();
        }
    }
}

/// Internal reason why `spawn_and_monitor` returned successfully.
enum ExitReason {
    /// OS signal or explicit shutdown command.
    ShutdownRequested,
    /// Manual restart via [`SupervisorCommand::Restart`].
    ManualRestart,
    /// Child exited with status 0 on its own.
    CleanExit,
}
