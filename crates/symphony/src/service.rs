use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::agent::ClaudeCodeAgent;
use crate::config::SymphonyConfig;
use crate::error::Result;
use crate::event::SymphonyEvent;
use crate::orchestrator::Orchestrator;
use crate::status::SymphonyStatusHandle;
use crate::tracker::GitHubIssueTracker;
use crate::workspace::WorkspaceManager;

/// Top-level service that wires together the symphony subsystem components
/// and runs the orchestrator event loop until shutdown.
pub struct SymphonyService {
    config: SymphonyConfig,
    shutdown: CancellationToken,
    github_token: Option<String>,
    status_handle: SymphonyStatusHandle,
}

impl SymphonyService {
    /// Create a new `SymphonyService`.
    ///
    /// # Arguments
    /// * `config` — symphony configuration
    /// * `shutdown` — cancellation token for graceful shutdown
    /// * `github_token` — optional GitHub PAT for API authentication
    #[must_use]
    pub fn new(
        config: SymphonyConfig,
        shutdown: CancellationToken,
        github_token: Option<String>,
    ) -> Self {
        let status_handle = SymphonyStatusHandle::new(&config);
        Self {
            config,
            shutdown,
            github_token,
            status_handle,
        }
    }

    /// Create a new `SymphonyService` with an externally-created status handle.
    ///
    /// Use this when the handle must exist before the service is created
    /// (e.g. to wire HTTP routes that are built before the service starts).
    #[must_use]
    pub fn with_status_handle(
        config: SymphonyConfig,
        shutdown: CancellationToken,
        github_token: Option<String>,
        status_handle: SymphonyStatusHandle,
    ) -> Self {
        Self {
            config,
            shutdown,
            github_token,
            status_handle,
        }
    }

    /// Return a clone of the status handle for use by HTTP routes.
    #[must_use]
    pub fn status_handle(&self) -> SymphonyStatusHandle {
        self.status_handle.clone()
    }

    /// Run the symphony service until shutdown is requested.
    pub async fn run(self) -> Result<()> {
        info!("starting symphony service");

        let tracker = Box::new(GitHubIssueTracker::new(
            self.config.repos.clone(),
            self.github_token,
        ));
        let workspace_mgr = WorkspaceManager::new(&self.config.repos);
        let agent = Box::new(ClaudeCodeAgent::new(self.config.agent.clone()));

        let mut orchestrator = Orchestrator::new(
            tracker,
            workspace_mgr,
            agent,
            self.config,
            self.status_handle,
        );
        let queue = orchestrator.queue().clone();

        tokio::select! {
            result = orchestrator.run() => result,
            _ = self.shutdown.cancelled() => {
                queue.push(SymphonyEvent::Shutdown);
                Ok(())
            }
        }
    }
}
