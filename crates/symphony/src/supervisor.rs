use std::{collections::HashMap, path::Path, time::Duration};

use tokio::process::{Child, Command};
use tracing::{info, warn};

use crate::{
    client::RalphClient,
    config::RepoConfig,
    error::{RalphSnafu, Result},
};

/// Base port — each repo gets BASE + index.
const BASE_PORT: u16 = 13781;

/// Default command for starting ralph.
const RALPH_COMMAND: &str = "ralph";

/// How long to wait between health check retries during startup.
const STARTUP_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Maximum time to wait for ralph to become healthy on startup.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

/// Delay before restarting a crashed ralph process.
const RESTART_DELAY: Duration = Duration::from_secs(3);

/// A single ralph instance managing one repo workspace.
struct RalphInstance {
    child:          Option<Child>,
    port:           u16,
    repo_name:      String,
    workspace_root: String,
    client:         RalphClient,
}

impl RalphInstance {
    fn new<P: AsRef<Path>>(repo_name: &str, workspace_root: P, port: u16) -> Self {
        let client = RalphClient::new(&format!("http://127.0.0.1:{port}"));
        Self {
            child: None,
            port,
            repo_name: repo_name.to_owned(),
            workspace_root: workspace_root.as_ref().to_string_lossy().into_owned(),
            client,
        }
    }

    async fn start(&mut self) -> Result<()> {
        info!(repo = %self.repo_name, port = self.port, "starting ralph");

        let child = Command::new(RALPH_COMMAND)
            .arg("web")
            .arg("--no-open")
            .arg("--backend-port")
            .arg(self.port.to_string())
            .arg("--workspace")
            .arg(&self.workspace_root)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| {
                RalphSnafu {
                    message: format!("failed to spawn ralph for {}: {e}", self.repo_name),
                }
                .build()
            })?;

        self.child = Some(child);

        let deadline = tokio::time::Instant::now() + STARTUP_TIMEOUT;
        loop {
            if tokio::time::Instant::now() > deadline {
                self.stop().await.ok();
                return Err(RalphSnafu {
                    message: format!(
                        "ralph for {} did not become healthy within {}s",
                        self.repo_name,
                        STARTUP_TIMEOUT.as_secs()
                    ),
                }
                .build());
            }

            if self.client.health().await {
                info!(repo = %self.repo_name, port = self.port, "ralph is healthy");
                return Ok(());
            }

            if let Some(ref mut child) = self.child {
                if let Ok(Some(status)) = child.try_wait() {
                    return Err(RalphSnafu {
                        message: format!(
                            "ralph for {} exited during startup with {status}",
                            self.repo_name
                        ),
                    }
                    .build());
                }
            }

            tokio::time::sleep(STARTUP_POLL_INTERVAL).await;
        }
    }

    async fn ensure_alive(&mut self) -> Result<()> {
        if self.client.health().await {
            return Ok(());
        }

        let needs_restart = match &mut self.child {
            Some(child) => matches!(child.try_wait(), Ok(Some(_))),
            None => true,
        };

        if needs_restart {
            warn!(repo = %self.repo_name, "ralph is not healthy, restarting after delay");
            tokio::time::sleep(RESTART_DELAY).await;
            self.start().await?;
        }

        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        if let Some(mut child) = self.child.take() {
            info!(repo = %self.repo_name, "stopping ralph");
            if let Err(e) = child.kill().await {
                warn!(repo = %self.repo_name, error = %e, "failed to kill ralph");
            }
            child.wait().await.ok();
        }
        Ok(())
    }
}

impl Drop for RalphInstance {
    fn drop(&mut self) {
        if let Some(ref mut child) = self.child {
            child.start_kill().ok();
        }
    }
}

/// Manages one ralph process per repo.
///
/// Each repo gets its own `ralph web` instance on a unique port,
/// since ralph is scoped to a single workspace root per server.
pub struct RalphSupervisor {
    instances: HashMap<String, RalphInstance>,
}

impl RalphSupervisor {
    /// Create a new supervisor from repo configs. Does not start processes yet.
    #[must_use]
    pub fn new(repos: &[RepoConfig]) -> Self {
        let mut instances = HashMap::new();
        for (i, repo) in repos.iter().enumerate() {
            let port = BASE_PORT + i as u16;
            // Use repo URL as workspace identifier for ralph.
            // In production the repo should be cloned locally; for now
            // we use the repo name as a stable workspace path under cwd.
            let workspace_root = rara_paths::config_dir()
                .join("ralph")
                .join(format!("workspaces/{}", repo.name.replace('/', "-")));
            instances.insert(
                repo.name.clone(),
                RalphInstance::new(&repo.name, workspace_root, port),
            );
        }
        Self { instances }
    }

    /// Return the ralph client for a specific repo.
    pub fn client(&self, repo_name: &str) -> Option<RalphClient> {
        self.instances.get(repo_name).map(|i| i.client.clone())
    }

    /// Start all ralph instances and wait until healthy.
    pub async fn start(&mut self) -> Result<()> {
        for instance in self.instances.values_mut() {
            instance.start().await?;
        }
        Ok(())
    }

    /// Ensure all ralph instances are alive, restarting any that crashed.
    pub async fn ensure_alive(&mut self) -> Result<()> {
        for instance in self.instances.values_mut() {
            instance.ensure_alive().await?;
        }
        Ok(())
    }

    /// Stop all ralph instances.
    pub async fn stop(&mut self) -> Result<()> {
        for instance in self.instances.values_mut() {
            instance.stop().await?;
        }
        Ok(())
    }
}
