use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use tracing::{error, info, warn};

use crate::agent::{AgentHandle, AgentTask, CodingAgent};
use crate::config::SymphonyConfig;
use crate::error::Result;
use crate::event::{IssueState, SymphonyEvent, TrackedIssue, WorkspaceInfo};
use crate::queue::EventQueue;
use crate::tracker::IssueTracker;
use crate::workspace::{run_hook, WorkspaceManager};

/// State of a running agent for a particular issue.
struct RunState {
    issue: TrackedIssue,
    workspace: WorkspaceInfo,
    #[allow(dead_code)]
    started_at: Instant,
    last_activity: Instant,
}

/// Retry tracking for a failed issue.
struct RetryEntry {
    attempt: u32,
}

/// The core event-loop driven orchestrator for symphony.
///
/// Polls issue trackers, dispatches coding agents, monitors for stalls,
/// and handles retries with exponential backoff.
pub struct Orchestrator {
    tracker: Box<dyn IssueTracker>,
    workspace_mgr: WorkspaceManager,
    agent: Box<dyn CodingAgent>,
    config: SymphonyConfig,
    queue: EventQueue,
    running: HashMap<String, RunState>,
    claimed: HashSet<String>,
    retries: HashMap<String, RetryEntry>,
}

impl Orchestrator {
    /// Create a new orchestrator.
    pub fn new(
        tracker: Box<dyn IssueTracker>,
        workspace_mgr: WorkspaceManager,
        agent: Box<dyn CodingAgent>,
        config: SymphonyConfig,
    ) -> Self {
        Self {
            tracker,
            workspace_mgr,
            agent,
            config,
            queue: EventQueue::new(),
            running: HashMap::new(),
            claimed: HashSet::new(),
            retries: HashMap::new(),
        }
    }

    /// Access the event queue, e.g. for external shutdown signaling.
    pub fn queue(&self) -> &EventQueue {
        &self.queue
    }

    /// Run the main event loop until a `Shutdown` event is received.
    pub async fn run(&mut self) -> Result<()> {
        // Seed the loop with an initial poll tick and stall check.
        self.queue.push(SymphonyEvent::PollTick);
        self.queue
            .schedule_after(self.config.stall_timeout, SymphonyEvent::StallCheck);

        loop {
            let event = self.queue.pop().await;

            match event {
                SymphonyEvent::Shutdown => {
                    info!("received shutdown signal, stopping orchestrator");
                    break;
                }
                SymphonyEvent::PollTick => {
                    if let Err(e) = self.handle_poll_tick().await {
                        error!(error = %e, "poll tick failed");
                    }
                }
                SymphonyEvent::IssueDiscovered { issue } => {
                    if let Err(e) = self.handle_dispatch(issue).await {
                        error!(error = %e, "dispatch failed");
                    }
                }
                SymphonyEvent::AgentCompleted { issue_id, workspace } => {
                    if let Err(e) = self.handle_agent_completed(&issue_id, &workspace).await {
                        error!(issue_id = %issue_id, error = %e, "agent completed handler failed");
                    }
                }
                SymphonyEvent::AgentFailed {
                    issue_id,
                    workspace: _,
                    reason,
                } => {
                    self.handle_agent_failed(&issue_id, &reason);
                }
                SymphonyEvent::AgentStalled { issue_id } => {
                    self.handle_agent_stalled(&issue_id);
                }
                SymphonyEvent::IssueStateChanged {
                    issue_id,
                    new_state,
                } => {
                    if let Err(e) = self.handle_state_changed(&issue_id, &new_state) {
                        error!(issue_id = %issue_id, error = %e, "state change handler failed");
                    }
                }
                SymphonyEvent::RetryReady { issue_id } => {
                    if let Err(e) = self.handle_retry(&issue_id).await {
                        error!(issue_id = %issue_id, error = %e, "retry handler failed");
                    }
                }
                SymphonyEvent::StallCheck => {
                    self.handle_stall_check();
                }
                SymphonyEvent::WorkspaceCleaned { issue_id, path } => {
                    info!(issue_id = %issue_id, path = %path.display(), "workspace cleaned");
                }
            }
        }

        Ok(())
    }

    // ── Handlers ─────────────────────────────────────────────────────

    /// Poll the tracker for active issues and discover new ones.
    async fn handle_poll_tick(&mut self) -> Result<()> {
        info!("polling for active issues");

        let issues = self.tracker.fetch_active_issues().await?;

        for issue in issues {
            if self.running.contains_key(&issue.id) || self.claimed.contains(&issue.id) {
                continue;
            }
            self.queue
                .push(SymphonyEvent::IssueDiscovered { issue });
        }

        // Schedule the next poll tick.
        self.queue
            .schedule_after(self.config.poll_interval, SymphonyEvent::PollTick);

        Ok(())
    }

    /// Dispatch an agent to work on a discovered issue.
    async fn handle_dispatch(&mut self, issue: TrackedIssue) -> Result<()> {
        let repo = &issue.repo;

        // Check capacity.
        if !self.has_global_slots() {
            info!(issue_id = %issue.id, "no global slots available, skipping");
            return Ok(());
        }
        if !self.has_repo_slots(repo) {
            info!(issue_id = %issue.id, repo = %repo, "no repo slots available, skipping");
            return Ok(());
        }
        if self.claimed.contains(&issue.id) {
            return Ok(());
        }

        // Claim the issue.
        self.claimed.insert(issue.id.clone());

        let repo_name = self.repo_name_for(&issue.repo);

        // Create or reuse a worktree.
        let workspace =
            self.workspace_mgr
                .ensure_worktree(&repo_name, issue.number, &issue.title)?;

        // Run lifecycle hooks.
        if let Some(hooks) = self.workspace_mgr.hooks_for(&repo_name) {
            if workspace.created_now {
                if let Some(script) = &hooks.after_create {
                    info!(issue_id = %issue.id, "running after_create hook");
                    if let Err(e) = run_hook(script, &workspace.path).await {
                        warn!(issue_id = %issue.id, error = %e, "after_create hook failed");
                    }
                }
            }
            if let Some(script) = &hooks.before_run {
                info!(issue_id = %issue.id, "running before_run hook");
                if let Err(e) = run_hook(script, &workspace.path).await {
                    warn!(issue_id = %issue.id, error = %e, "before_run hook failed");
                }
            }
        }

        // Read WORKFLOW.md from the workspace if it exists.
        let workflow_file = self.workflow_file_for(&repo_name);
        let workflow_path = workspace.path.join(&workflow_file);
        let workflow_content = std::fs::read_to_string(&workflow_path).ok();

        // Determine retry attempt from retry tracking.
        let attempt = self.retries.get(&issue.id).map(|r| r.attempt);

        // Build the agent task.
        let task = AgentTask {
            issue: issue.clone(),
            attempt,
            workflow_content,
        };

        // Start the agent.
        let handle: AgentHandle = self.agent.start(&task, &workspace).await?;

        // Take ownership of the child process for the watcher.
        let AgentHandle {
            child,
            started_at,
        } = handle;

        let issue_id = issue.id.clone();
        let ws_clone = workspace.clone();
        let queue = self.queue.clone();

        // Spawn a watcher task to monitor the child process.
        tokio::spawn(async move {
            match child.wait_with_output().await {
                Ok(output) if output.status.success() => {
                    queue.push(SymphonyEvent::AgentCompleted {
                        issue_id,
                        workspace: ws_clone,
                    });
                }
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let reason = format!(
                        "exit code {:?}: {}",
                        output.status.code(),
                        stderr.trim()
                    );
                    queue.push(SymphonyEvent::AgentFailed {
                        issue_id,
                        workspace: ws_clone,
                        reason,
                    });
                }
                Err(e) => {
                    queue.push(SymphonyEvent::AgentFailed {
                        issue_id,
                        workspace: ws_clone,
                        reason: format!("failed to wait on child: {e}"),
                    });
                }
            }
        });

        // Store the run state (without the child, which has been moved).
        let now = Instant::now();
        self.running.insert(
            issue.id.clone(),
            RunState {
                issue,
                workspace,
                started_at,
                last_activity: now,
            },
        );

        Ok(())
    }

    /// Handle a successfully completed agent.
    async fn handle_agent_completed(
        &mut self,
        issue_id: &str,
        workspace: &WorkspaceInfo,
    ) -> Result<()> {
        info!(issue_id = %issue_id, "agent completed successfully");

        let run_state = self.running.remove(issue_id);
        self.claimed.remove(issue_id);
        self.retries.remove(issue_id);

        // Run after_run hook.
        if let Some(rs) = &run_state {
            let repo_name = self.repo_name_for(&rs.issue.repo);
            if let Some(hooks) = self.workspace_mgr.hooks_for(&repo_name) {
                if let Some(script) = &hooks.after_run {
                    info!(issue_id = %issue_id, "running after_run hook");
                    if let Err(e) = run_hook(script, &workspace.path).await {
                        warn!(issue_id = %issue_id, error = %e, "after_run hook failed");
                    }
                }
            }

            // Cleanup the worktree.
            if let Err(e) = self.workspace_mgr.cleanup_worktree(&repo_name, workspace) {
                warn!(issue_id = %issue_id, error = %e, "worktree cleanup failed");
            }
        }

        self.queue.push(SymphonyEvent::WorkspaceCleaned {
            issue_id: issue_id.to_owned(),
            path: workspace.path.clone(),
        });

        Ok(())
    }

    /// Handle a failed agent — compute retry backoff and schedule a retry.
    fn handle_agent_failed(&mut self, issue_id: &str, reason: &str) {
        warn!(issue_id = %issue_id, reason = %reason, "agent failed");

        self.running.remove(issue_id);

        let entry = self
            .retries
            .entry(issue_id.to_owned())
            .or_insert(RetryEntry { attempt: 0 });
        entry.attempt += 1;
        let attempt = entry.attempt;

        let backoff = self.compute_backoff(attempt);
        info!(
            issue_id = %issue_id,
            attempt = attempt,
            backoff_secs = backoff.as_secs(),
            "scheduling retry"
        );

        self.queue.schedule_after(
            backoff,
            SymphonyEvent::RetryReady {
                issue_id: issue_id.to_owned(),
            },
        );
    }

    /// Treat a stalled agent as a failure.
    fn handle_agent_stalled(&mut self, issue_id: &str) {
        warn!(issue_id = %issue_id, "agent stalled, treating as failure");
        self.handle_agent_failed(issue_id, "agent stalled (no activity)");
    }

    /// Handle an issue state change — if terminal, cleanup.
    fn handle_state_changed(&mut self, issue_id: &str, new_state: &IssueState) -> Result<()> {
        info!(issue_id = %issue_id, state = ?new_state, "issue state changed");

        if *new_state == IssueState::Terminal {
            if let Some(run_state) = self.running.remove(issue_id) {
                self.claimed.remove(issue_id);
                self.retries.remove(issue_id);

                let repo_name = self.repo_name_for(&run_state.issue.repo);
                if let Err(e) = self
                    .workspace_mgr
                    .cleanup_worktree(&repo_name, &run_state.workspace)
                {
                    warn!(issue_id = %issue_id, error = %e, "worktree cleanup failed");
                }

                self.queue.push(SymphonyEvent::WorkspaceCleaned {
                    issue_id: issue_id.to_owned(),
                    path: run_state.workspace.path,
                });
            } else {
                // Not running — just clean up tracking state.
                self.claimed.remove(issue_id);
                self.retries.remove(issue_id);
            }
        }

        Ok(())
    }

    /// Handle a retry — re-check issue state and re-dispatch if still active.
    async fn handle_retry(&mut self, issue_id: &str) -> Result<()> {
        info!(issue_id = %issue_id, "processing retry");

        // Remove from claimed so dispatch can re-claim.
        self.claimed.remove(issue_id);
        self.running.remove(issue_id);

        // Parse repo and number from issue_id (format: "owner/repo#number").
        let (repo, number) = match issue_id.rsplit_once('#') {
            Some((r, n)) => match n.parse::<u64>() {
                Ok(num) => (r, num),
                Err(_) => {
                    warn!(issue_id = %issue_id, "cannot parse issue number from id");
                    return Ok(());
                }
            },
            None => {
                warn!(issue_id = %issue_id, "cannot parse repo from issue id");
                return Ok(());
            }
        };

        // Re-check issue state.
        match self.tracker.fetch_issue_state(repo, number).await {
            Ok(IssueState::Active) => {
                info!(issue_id = %issue_id, "issue still active, re-dispatching");
                // We need to fetch the full issue again. Trigger a poll to pick it up.
                // Or we could fetch active issues — for simplicity, just push a PollTick.
                self.queue.push(SymphonyEvent::PollTick);
            }
            Ok(IssueState::Terminal) => {
                info!(issue_id = %issue_id, "issue is now terminal, dropping retry");
                self.retries.remove(issue_id);
            }
            Err(e) => {
                warn!(issue_id = %issue_id, error = %e, "failed to fetch issue state for retry");
            }
        }

        Ok(())
    }

    /// Check all running agents for stalls.
    fn handle_stall_check(&mut self) {
        let stall_timeout = self.config.stall_timeout;
        let stalled: Vec<String> = self
            .running
            .iter()
            .filter(|(_, rs)| rs.last_activity.elapsed() > stall_timeout)
            .map(|(id, _)| id.clone())
            .collect();

        for issue_id in stalled {
            self.queue
                .push(SymphonyEvent::AgentStalled { issue_id });
        }

        // Schedule the next stall check.
        self.queue
            .schedule_after(stall_timeout, SymphonyEvent::StallCheck);
    }

    // ── Helpers ──────────────────────────────────────────────────────

    /// Check if there are global slots available.
    fn has_global_slots(&self) -> bool {
        self.running.len() < self.config.max_concurrent_agents
    }

    /// Check if there are repo-level slots available for the given repo.
    fn has_repo_slots(&self, repo: &str) -> bool {
        let repo_name = self.repo_name_for(repo);

        // Find per-repo max from config.
        let max = self
            .config
            .repos
            .iter()
            .find(|r| r.name == repo_name)
            .and_then(|r| r.max_concurrent_agents)
            .unwrap_or(self.config.max_concurrent_agents);

        let current = self
            .running
            .values()
            .filter(|rs| self.repo_name_for(&rs.issue.repo) == repo_name)
            .count();

        current < max
    }

    /// Extract the repo name from a repo slug.
    fn repo_name_for(&self, repo_slug: &str) -> String {
        repo_slug.to_owned()
    }

    /// Get the workflow file name for a given repo.
    fn workflow_file_for(&self, repo_name: &str) -> String {
        self.config
            .repos
            .iter()
            .find(|r| r.name == repo_name)
            .and_then(|r| r.workflow_file.clone())
            .unwrap_or_else(|| self.config.workflow_file.clone())
    }

    /// Compute exponential backoff: min(10s * 2^(attempt-1), max_retry_backoff).
    fn compute_backoff(&self, attempt: u32) -> Duration {
        let base = Duration::from_secs(10);
        let multiplier = 2u64.saturating_pow(attempt.saturating_sub(1));
        let backoff = base.saturating_mul(multiplier as u32);
        std::cmp::min(backoff, self.config.max_retry_backoff)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AgentConfig, HooksConfig, RepoConfig};
    use crate::event::IssueState;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    /// A mock tracker that returns no issues and records whether it was polled.
    struct MockTracker {
        polled: Arc<AtomicBool>,
    }

    #[async_trait]
    impl IssueTracker for MockTracker {
        async fn fetch_active_issues(&self) -> Result<Vec<TrackedIssue>> {
            self.polled.store(true, Ordering::SeqCst);
            Ok(vec![])
        }

        async fn fetch_issue_state(&self, _repo: &str, _number: u64) -> Result<IssueState> {
            Ok(IssueState::Active)
        }
    }

    /// A mock agent that is never actually called in this test.
    struct MockAgent;

    #[async_trait]
    impl CodingAgent for MockAgent {
        async fn start(
            &self,
            _task: &AgentTask,
            _workspace: &WorkspaceInfo,
        ) -> Result<crate::agent::AgentHandle> {
            unreachable!("should not be called in smoke test");
        }
    }

    #[tokio::test]
    async fn smoke_orchestrator_polls_and_shuts_down() {
        let polled = Arc::new(AtomicBool::new(false));

        let config = SymphonyConfig::builder()
            .enabled(true)
            .poll_interval(Duration::from_secs(60))
            .max_concurrent_agents(2)
            .stall_timeout(Duration::from_secs(300))
            .max_retry_backoff(Duration::from_secs(600))
            .workflow_file("WORKFLOW.md".to_owned())
            .agent(
                AgentConfig::builder()
                    .command("echo".to_owned())
                    .args(vec![])
                    .allowed_tools(vec![])
                    .build(),
            )
            .repos(vec![RepoConfig::builder()
                .name("test/repo".to_owned())
                .url("https://example.com".to_owned())
                .repo_path("/tmp/repo".into())
                .workspace_root("/tmp/ws".into())
                .active_labels(vec!["symphony:ready".to_owned()])
                .hooks(HooksConfig::default())
                .build()])
            .build();

        let tracker = MockTracker {
            polled: polled.clone(),
        };
        let workspace_mgr = WorkspaceManager::new(&config.repos);
        let agent = MockAgent;

        let mut orchestrator = Orchestrator::new(
            Box::new(tracker),
            workspace_mgr,
            Box::new(agent),
            config,
        );

        // Schedule a shutdown after a short delay.
        orchestrator
            .queue()
            .schedule_after(Duration::from_millis(200), SymphonyEvent::Shutdown);

        orchestrator.run().await.expect("orchestrator should run");

        assert!(
            polled.load(Ordering::SeqCst),
            "tracker should have been polled"
        );
    }

    #[test]
    fn compute_backoff_exponential() {
        let config = SymphonyConfig::builder()
            .enabled(true)
            .poll_interval(Duration::from_secs(60))
            .max_concurrent_agents(2)
            .stall_timeout(Duration::from_secs(300))
            .max_retry_backoff(Duration::from_secs(120))
            .workflow_file("WORKFLOW.md".to_owned())
            .agent(
                AgentConfig::builder()
                    .command("echo".to_owned())
                    .args(vec![])
                    .allowed_tools(vec![])
                    .build(),
            )
            .repos(vec![])
            .build();

        let orchestrator = Orchestrator::new(
            Box::new(MockTracker {
                polled: Arc::new(AtomicBool::new(false)),
            }),
            WorkspaceManager::new(&[]),
            Box::new(MockAgent),
            config,
        );

        // 10s * 2^0 = 10s
        assert_eq!(orchestrator.compute_backoff(1), Duration::from_secs(10));
        // 10s * 2^1 = 20s
        assert_eq!(orchestrator.compute_backoff(2), Duration::from_secs(20));
        // 10s * 2^2 = 40s
        assert_eq!(orchestrator.compute_backoff(3), Duration::from_secs(40));
        // 10s * 2^3 = 80s
        assert_eq!(orchestrator.compute_backoff(4), Duration::from_secs(80));
        // 10s * 2^4 = 160s → capped at 120s
        assert_eq!(orchestrator.compute_backoff(5), Duration::from_secs(120));
    }
}
