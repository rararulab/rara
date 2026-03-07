use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::agent::ClaudeCodeAgent;
use crate::config::{SymphonyConfig, TrackerConfig};
use crate::error::Result;
use crate::event::SymphonyEvent;
use crate::orchestrator::Orchestrator;
use crate::status::SymphonyStatusHandle;
use crate::tracker::{GitHubIssueTracker, LinearIssueTracker};
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

        let tracker: Box<dyn crate::tracker::IssueTracker> = match &self.config.tracker {
            Some(TrackerConfig::Linear {
                api_key,
                project_slug,
                endpoint,
                active_states,
                terminal_states,
                repo_label_prefix,
            }) => {
                let resolved_key = resolve_env_var(api_key)?;
                let repo_names = self.config.repos.iter().map(|r| r.name.clone()).collect();
                Box::new(LinearIssueTracker::new(
                    &resolved_key,
                    endpoint,
                    project_slug.clone(),
                    active_states.clone(),
                    terminal_states.clone(),
                    repo_label_prefix.clone(),
                    repo_names,
                )?)
            }
            Some(TrackerConfig::Github { api_key }) => {
                let token = match api_key {
                    Some(k) => Some(resolve_env_var(k)?),
                    None => self.github_token.clone(),
                };
                Box::new(GitHubIssueTracker::new(
                    self.config.repos.clone(),
                    token,
                ))
            }
            None => Box::new(GitHubIssueTracker::new(
                self.config.repos.clone(),
                self.github_token.clone(),
            )),
        };
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

/// Resolve a `$ENV_VAR` reference to its value, or return the string as-is.
fn resolve_env_var(value: &str) -> crate::error::Result<String> {
    if let Some(var_name) = value.strip_prefix('$') {
        std::env::var(var_name).map_err(|_| {
            crate::error::ConfigSnafu {
                message: format!("environment variable '{var_name}' not set"),
            }
            .build()
        })
    } else {
        Ok(value.to_owned())
    }
}
