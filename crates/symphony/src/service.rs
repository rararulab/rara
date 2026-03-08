use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::config::{SymphonyConfig, TrackerConfig};
use crate::error::Result;
use crate::supervisor::RalphSupervisor;
use crate::syncer::IssueSyncer;
use crate::tracker::{GitHubIssueTracker, IssueTracker, LinearIssueTracker};

/// Top-level service that bridges issue trackers with ralph's task API.
///
/// Spawns and supervises a ralph-api process, then runs a poll loop that
/// syncs issues to ralph tasks.
pub struct SymphonyService {
    config: SymphonyConfig,
    shutdown: CancellationToken,
    github_token: Option<String>,
}

impl SymphonyService {
    #[must_use]
    pub fn new(
        config: SymphonyConfig,
        shutdown: CancellationToken,
        github_token: Option<String>,
    ) -> Self {
        Self {
            config,
            shutdown,
            github_token,
        }
    }

    /// Run the symphony service until shutdown is requested.
    pub async fn run(self) -> Result<()> {
        info!("starting symphony service");

        // 1. Build issue tracker.
        let tracker: Box<dyn IssueTracker> = self.build_tracker()?;

        // 2. Start ralph-api supervisor.
        let mut supervisor = RalphSupervisor::new();
        supervisor.start().await?;
        let syncer = IssueSyncer::new(supervisor.client());

        info!("symphony sync loop started");

        // 3. Poll loop.
        loop {
            tokio::select! {
                _ = self.shutdown.cancelled() => {
                    info!("symphony shutting down");
                    break;
                }
                _ = tokio::time::sleep(self.config.poll_interval) => {
                    self.poll_cycle(&*tracker, &syncer, &mut supervisor).await;
                }
            }
        }

        // 4. Shutdown.
        supervisor.stop().await?;
        info!("symphony service stopped");
        Ok(())
    }

    /// Run one poll cycle: ensure ralph is alive, fetch issues, sync.
    async fn poll_cycle(
        &self,
        tracker: &dyn IssueTracker,
        syncer: &IssueSyncer,
        supervisor: &mut RalphSupervisor,
    ) {
        // Ensure ralph-api is running.
        if let Err(e) = supervisor.ensure_alive().await {
            error!(error = %e, "failed to ensure ralph-api is alive, skipping cycle");
            return;
        }

        // Fetch active issues.
        let issues = match tracker.fetch_active_issues().await {
            Ok(issues) => issues,
            Err(e) => {
                warn!(error = %e, "failed to fetch issues, skipping cycle");
                return;
            }
        };

        // Sync.
        match syncer.sync(tracker, &issues).await {
            Ok(report) => {
                if !report.created.is_empty()
                    || !report.completed.is_empty()
                    || !report.cancelled.is_empty()
                    || !report.failed.is_empty()
                {
                    info!(
                        created = report.created.len(),
                        completed = report.completed.len(),
                        cancelled = report.cancelled.len(),
                        failed = report.failed.len(),
                        unchanged = report.unchanged,
                        "sync cycle completed"
                    );
                }
            }
            Err(e) => {
                warn!(error = %e, "sync cycle failed");
            }
        }
    }

    /// Build the issue tracker from config.
    fn build_tracker(&self) -> Result<Box<dyn IssueTracker>> {
        match &self.config.tracker {
            Some(TrackerConfig::Linear {
                api_key,
                team_key,
                project_slug,
                endpoint,
                active_states,
                terminal_states,
                repo_label_prefix,
            }) => {
                let resolved_key = resolve_env_var(api_key)?;
                let repo_names = self.config.repos.iter().map(|r| r.name.clone()).collect();
                Ok(Box::new(LinearIssueTracker::new(
                    &resolved_key,
                    endpoint,
                    team_key.clone(),
                    project_slug.clone(),
                    active_states.clone(),
                    terminal_states.clone(),
                    repo_label_prefix.clone(),
                    repo_names,
                )?))
            }
            Some(TrackerConfig::Github { api_key }) => {
                let token = match api_key {
                    Some(k) => Some(resolve_env_var(k)?),
                    None => self.github_token.clone(),
                };
                Ok(Box::new(GitHubIssueTracker::new(
                    self.config.repos.clone(),
                    token,
                )))
            }
            None => Ok(Box::new(GitHubIssueTracker::new(
                self.config.repos.clone(),
                self.github_token.clone(),
            ))),
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
