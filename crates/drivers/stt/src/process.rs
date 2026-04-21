//! Child-process supervisor for a managed whisper-server instance.

use std::{path::PathBuf, process::Stdio, time::Duration};

use tokio::{
    process::{Child, Command},
    task::JoinHandle,
    time::sleep,
};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, instrument, warn};
use url::Url;

use super::SttConfig;

/// Maximum time to wait for the whisper-server `/health` endpoint to become
/// reachable after spawning the process.
const HEALTH_TIMEOUT: Duration = Duration::from_mins(1);

/// Interval between consecutive health-check polls during startup.
const HEALTH_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Delay before restarting a crashed child process.
const RESTART_DELAY: Duration = Duration::from_secs(2);

/// Per-request timeout for the HTTP health check.
const HEALTH_REQUEST_TIMEOUT: Duration = Duration::from_secs(2);

/// Supervisor for a managed whisper-server child process.
///
/// When `managed: true` in [`SttConfig`], rara spawns and supervises the
/// whisper-server binary itself rather than expecting an external instance.
/// The supervisor monitors the child and restarts it automatically on crash.
pub struct WhisperProcess {
    server_bin: PathBuf,
    model_path: PathBuf,
    host:       String,
    port:       u16,
    child:      Option<Child>,
}

impl WhisperProcess {
    /// Build a supervisor from config, returning `None` when managed mode is
    /// disabled or the required paths are missing.
    pub fn from_config(config: &SttConfig) -> Option<Self> {
        if !config.managed {
            return None;
        }

        let server_bin = config.server_bin.clone()?;
        let model_path = config.model_path.clone()?;

        let parsed = Url::parse(&config.base_url)
            .map_err(|e| warn!("failed to parse stt base_url: {e}"))
            .ok()?;

        let host = parsed.host_str().unwrap_or("127.0.0.1").to_owned();
        let port = parsed.port().unwrap_or(8080);

        Some(Self {
            server_bin,
            model_path,
            host,
            port,
            child: None,
        })
    }

    /// Spawn the whisper-server child process and block until its `/health`
    /// endpoint responds (up to 60 s).
    #[instrument(skip_all, fields(bin = %self.server_bin.display()))]
    pub async fn start(&mut self) -> anyhow::Result<()> {
        let child = Command::new(&self.server_bin)
            .arg("-m")
            .arg(&self.model_path)
            .arg("--host")
            .arg(&self.host)
            .arg("--port")
            .arg(self.port.to_string())
            .arg("--inference-path")
            .arg("/v1/audio/transcriptions")
            .arg("--convert")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        info!(
            pid = child.id().unwrap_or(0),
            "whisper-server process spawned"
        );
        self.child = Some(child);

        self.wait_healthy().await?;
        info!("whisper-server is healthy");
        Ok(())
    }

    /// Gracefully stop the child process if it is running.
    #[instrument(skip_all)]
    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let pid = child.id().unwrap_or(0);
            // start_kill sends SIGKILL on Unix; best-effort
            if let Err(e) = child.start_kill() {
                warn!(pid, "failed to kill whisper-server: {e}");
            } else {
                info!(pid, "whisper-server stopped");
            }
        }
    }

    /// Spawn a background supervisor task that watches the child process,
    /// restarting it on crash with a brief delay. The loop exits when the
    /// supplied [`CancellationToken`] is cancelled.
    pub fn spawn_supervisor(mut self, cancel: CancellationToken) -> JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                // Start the child if not already running.
                if self.child.is_none() {
                    if let Err(e) = self.start().await {
                        error!("failed to start whisper-server: {e}");
                        // Wait before retry to avoid tight crash loops.
                        tokio::select! {
                            () = cancel.cancelled() => break,
                            () = sleep(RESTART_DELAY) => continue,
                        }
                    }
                }

                // Wait for either the child to exit or cancellation.
                tokio::select! {
                    () = cancel.cancelled() => {
                        info!("supervisor cancelled, stopping whisper-server");
                        self.stop();
                        break;
                    }
                    status = Self::watch_child(&mut self.child) => {
                        warn!("whisper-server exited: {status}");
                        self.child = None;
                        // Brief delay before restart.
                        tokio::select! {
                            () = cancel.cancelled() => break,
                            () = sleep(RESTART_DELAY) => {}
                        }
                    }
                }
            }
        })
    }

    // ------------------------------------------------------------------
    // Private helpers
    // ------------------------------------------------------------------

    /// Poll the `/health` endpoint until it returns a success status.
    async fn wait_healthy(&self) -> anyhow::Result<()> {
        let url = format!("http://{}:{}/health", self.host, self.port);
        let client = reqwest::Client::builder()
            .timeout(HEALTH_REQUEST_TIMEOUT)
            .build()?;

        let deadline = tokio::time::Instant::now() + HEALTH_TIMEOUT;

        loop {
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!(
                    "whisper-server did not become healthy within {}s",
                    HEALTH_TIMEOUT.as_secs()
                );
            }
            match client.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => return Ok(()),
                Ok(resp) => {
                    warn!(status = %resp.status(), "health check returned non-success");
                }
                Err(e) => {
                    // Expected while the server is still starting up.
                    tracing::trace!("health check failed: {e}");
                }
            }
            sleep(HEALTH_POLL_INTERVAL).await;
        }
    }

    /// Await termination of the child process. Returns the exit status as a
    /// human-readable string.
    async fn watch_child(child: &mut Option<Child>) -> String {
        match child {
            Some(c) => match c.wait().await {
                Ok(status) => status.to_string(),
                Err(e) => format!("error waiting for child: {e}"),
            },
            None => {
                // Should not happen — caller ensures child is Some.
                futures::future::pending::<()>().await;
                unreachable!()
            }
        }
    }
}
