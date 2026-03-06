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

use std::time::{Duration, Instant};

use snafu::{ResultExt, Snafu};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
};
use tracing::{error, info, warn};

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
// SupervisorService
// ---------------------------------------------------------------------------

/// Manages a single `rara server` child process with health checks and
/// restart-with-backoff semantics.
pub struct SupervisorService {
    config:           GatewayConfig,
    health_url:       String,
    child:            Option<Child>,
    restart_count:    u32,
    last_healthy:     Option<Instant>,
}

impl SupervisorService {
    /// Create a new supervisor.
    ///
    /// `health_port` is the HTTP port the agent binds to (from
    /// `RestServerConfig::bind_address`).
    pub fn new(config: GatewayConfig, health_port: &str) -> Self {
        let health_url = format!("http://127.0.0.1:{health_port}/api/health");
        Self {
            config,
            health_url,
            child: None,
            restart_count: 0,
            last_healthy: None,
        }
    }

    /// Run the supervisor loop until a shutdown signal is received or
    /// max restarts are exhausted.
    ///
    /// This is the main entry point — it spawns the child, monitors it,
    /// and restarts on failure with exponential backoff.
    pub async fn run(&mut self) -> Result<(), SupervisorError> {
        loop {
            // Reset restart counter if healthy for long enough.
            if let Some(ts) = self.last_healthy {
                if ts.elapsed() >= Duration::from_secs(60) {
                    if self.restart_count > 0 {
                        info!("Agent healthy for 60s — resetting restart counter");
                    }
                    self.restart_count = 0;
                }
            }

            info!(attempt = self.restart_count + 1, "Spawning agent process");

            match self.spawn_and_monitor().await {
                Ok(()) => {
                    // Child exited cleanly (status 0). This is unexpected for
                    // a long-running server but not an error — restart it.
                    info!("Agent process exited cleanly, restarting");
                }
                Err(e) => {
                    warn!(error = %e, "Agent process failed");
                    self.restart_count += 1;

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
                    info!(backoff_secs = backoff.as_secs(), "Backing off before restart");

                    tokio::select! {
                        () = tokio::time::sleep(backoff) => {}
                        () = Self::shutdown_signal() => {
                            info!("Shutdown signal received during backoff");
                            return Ok(());
                        }
                    }
                }
            }
        }
    }

    /// Spawn the child, run health checks, then wait for it to exit or
    /// a shutdown signal.
    async fn spawn_and_monitor(&mut self) -> Result<(), SupervisorError> {
        let mut child = self.spawn_child().await?;

        // Phase 1: wait for READY on stdout.
        let stdout = child.stdout.take();
        self.wait_for_ready(stdout).await?;

        // Phase 2: HTTP health polling.
        self.poll_health().await?;

        info!("Agent is healthy");
        self.last_healthy = Some(Instant::now());
        self.child = Some(child);

        // Wait for the child to exit or a shutdown signal.
        tokio::select! {
            status = self.child.as_mut().unwrap().wait() => {
                match status {
                    Ok(s) if s.success() => {
                        info!("Agent process exited with status 0");
                        self.child = None;
                        Ok(())
                    }
                    Ok(s) => {
                        self.child = None;
                        warn!(status = %s, "Agent process exited with non-zero status");
                        Err(SupervisorError::Spawn {
                            source: std::io::Error::new(
                                std::io::ErrorKind::Other,
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
            () = Self::shutdown_signal() => {
                info!("Shutdown signal received — stopping agent");
                self.graceful_shutdown().await;
                Ok(())
            }
        }
    }

    /// Spawn `rara server` as a child process.
    async fn spawn_child(&self) -> Result<Child, SupervisorError> {
        let exe = std::env::current_exe().context(SpawnSnafu)?;
        Command::new(&exe)
            .arg("server")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .context(SpawnSnafu)
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

        let result = tokio::time::timeout(timeout, async {
            while let Ok(Some(line)) = reader.next_line().await {
                if line.contains("READY") {
                    return true;
                }
            }
            false
        })
        .await;

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

    /// Phase 2: HTTP GET `/api/health` — 3 consecutive 200s required.
    async fn poll_health(&self) -> Result<(), SupervisorError> {
        let timeout = Duration::from_secs(self.config.health_timeout / 2);
        let poll_interval = Duration::from_secs(self.config.health_poll_interval);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_default();

        let mut consecutive_ok = 0u32;
        let deadline = Instant::now() + timeout;

        while Instant::now() < deadline {
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
            tokio::time::sleep(poll_interval).await;
        }

        Err(SupervisorError::HealthTimeout {
            timeout_secs: self.config.health_timeout / 2,
        })
    }

    /// Graceful shutdown: SIGTERM → wait 5s → SIGKILL.
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

        info!(pid, "Sending SIGTERM to agent");

        #[cfg(unix)]
        {
            // Use the `kill` command to send SIGTERM without requiring `unsafe`.
            let _ = std::process::Command::new("kill")
                .args(["-TERM", &pid.to_string()])
                .status();
        }

        // Wait up to 5 seconds for clean exit.
        match tokio::time::timeout(Duration::from_secs(5), child.wait()).await {
            Ok(Ok(status)) => {
                info!(%status, "Agent exited after SIGTERM");
            }
            _ => {
                warn!(pid, "Agent did not exit after 5s — sending SIGKILL");
                let _ = child.kill().await;
            }
        }

        self.child = None;
    }

    /// Wait for SIGTERM or SIGINT.
    async fn shutdown_signal() {
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
