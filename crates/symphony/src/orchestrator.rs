use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use chrono::Utc;
use tracing::{error, info, warn};

use crate::agent::{AgentHandle, AgentTask, CodingAgent};
use crate::config::SymphonyConfig;
use crate::error::Result;
use crate::event::{IssueState, SymphonyEvent, TrackedIssue, WorkspaceInfo};
use crate::queue::EventQueue;
use crate::status::{
    ConfigSummary, RetryInfo, RunInfo, SymphonyEventLog, SymphonySnapshot, SymphonyStatusHandle,
};
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
    issue: TrackedIssue,
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
    status_handle: SymphonyStatusHandle,
}

impl Orchestrator {
    /// Create a new orchestrator.
    pub fn new(
        tracker: Box<dyn IssueTracker>,
        workspace_mgr: WorkspaceManager,
        agent: Box<dyn CodingAgent>,
        config: SymphonyConfig,
        status_handle: SymphonyStatusHandle,
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
            status_handle,
        }
    }

    /// Access the event queue, e.g. for external shutdown signaling.
    pub fn queue(&self) -> &EventQueue {
        &self.queue
    }

    /// Run the main event loop until a `Shutdown` event is received.
    ///
    /// Uses `tokio::select!` to multiplex periodic timers (poll, stall check)
    /// with async events from the queue (agent completion, failures, retries).
    pub async fn run(&mut self) -> Result<()> {
        let mut poll_interval = tokio::time::interval(self.config.poll_interval);
        let mut stall_interval = tokio::time::interval(self.config.stall_timeout);

        // First tick fires immediately for poll, skip for stall (wait one full period first).
        poll_interval.tick().await;
        stall_interval.tick().await;

        loop {
            tokio::select! {
                _ = poll_interval.tick() => {
                    self.log_event("poll_tick", None, "polling for active issues");
                    if let Err(e) = self.handle_poll_tick().await {
                        error!(error = %e, "poll tick failed");
                    }
                    self.publish_snapshot().await;
                }
                _ = stall_interval.tick() => {
                    self.log_event("stall_check", None, "checking for stalled agents");
                    self.handle_stall_check();
                    self.publish_snapshot().await;
                }
                event = self.queue.pop() => {
                    self.log_event_for(&event);
                    match event {
                        SymphonyEvent::Shutdown => {
                            info!("received shutdown signal, stopping orchestrator");
                            self.publish_snapshot().await;
                            break;
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
                        SymphonyEvent::AgentFailed { issue_id, workspace: _, reason } => {
                            self.handle_agent_failed(&issue_id, &reason);
                        }
                        SymphonyEvent::AgentStalled { issue_id } => {
                            self.handle_agent_stalled(&issue_id);
                        }
                        SymphonyEvent::IssueStateChanged { issue_id, new_state } => {
                            if let Err(e) = self.handle_state_changed(&issue_id, &new_state).await {
                                error!(issue_id = %issue_id, error = %e, "state change handler failed");
                            }
                        }
                        SymphonyEvent::RetryReady { issue_id } => {
                            if let Err(e) = self.handle_retry(&issue_id).await {
                                error!(issue_id = %issue_id, error = %e, "retry handler failed");
                            }
                        }
                        SymphonyEvent::WorkspaceCleaned { issue_id, path } => {
                            info!(issue_id = %issue_id, path = %path.display(), "workspace cleaned");
                        }
                    }
                    self.publish_snapshot().await;
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

        let new_count = issues.iter()
            .filter(|i| !self.running.contains_key(&i.id) && !self.claimed.contains(&i.id))
            .count();
        info!(
            total = issues.len(),
            new = new_count,
            running = self.running.len(),
            claimed = self.claimed.len(),
            "poll: fetched issues"
        );

        for issue in issues {
            if self.running.contains_key(&issue.id) || self.claimed.contains(&issue.id) {
                continue;
            }
            self.queue
                .push(SymphonyEvent::IssueDiscovered { issue });
        }

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

        // Run the actual dispatch logic; on ANY error, release the claim
        // so the issue can be retried on a future poll tick.
        match self.do_dispatch(issue.clone()).await {
            Ok(()) => Ok(()),
            Err(e) => {
                error!(
                    issue_id = %issue.id,
                    error = %e,
                    "dispatch failed, releasing claim"
                );
                self.claimed.remove(&issue.id);
                Err(e)
            }
        }
    }

    /// Inner dispatch logic, separated so that `handle_dispatch` can clean up
    /// the `claimed` set on error.
    async fn do_dispatch(&mut self, issue: TrackedIssue) -> Result<()> {
        let repo_name = self.repo_name_for(&issue.repo);

        // Create or reuse a worktree (blocking git2 I/O — offload to spawn_blocking).
        let mgr = self.workspace_mgr.clone();
        let rn = repo_name.clone();
        let issue_number = issue.number;
        let issue_title = issue.title.clone();
        let workspace = tokio::task::spawn_blocking(move || {
            mgr.ensure_worktree(&rn, issue_number, &issue_title)
        })
        .await
        .expect("spawn_blocking panicked")?;

        info!(
            issue_id = %issue.id,
            identifier = %issue.identifier,
            repo = %issue.repo,
            title = %issue.title,
            workspace = %workspace.path.display(),
            branch = %workspace.branch,
            "dispatching agent"
        );

        // Run lifecycle hooks (with timeout to prevent hanging the event loop).
        let hook_timeout = Duration::from_secs(60);
        if let Some(hooks) = self.workspace_mgr.hooks_for(&repo_name) {
            if workspace.created_now {
                if let Some(script) = &hooks.after_create {
                    info!(issue_id = %issue.id, "running after_create hook");
                    match tokio::time::timeout(hook_timeout, run_hook(script, &workspace.path)).await {
                        Ok(Ok(())) => info!(issue_id = %issue.id, "after_create hook completed"),
                        Ok(Err(e)) => warn!(issue_id = %issue.id, error = %e, "after_create hook failed"),
                        Err(_) => warn!(issue_id = %issue.id, "after_create hook timed out after 60s"),
                    }
                }
            }
            if let Some(script) = &hooks.before_run {
                info!(issue_id = %issue.id, "running before_run hook");
                match tokio::time::timeout(hook_timeout, run_hook(script, &workspace.path)).await {
                    Ok(Ok(())) => info!(issue_id = %issue.id, "before_run hook completed"),
                    Ok(Err(e)) => warn!(issue_id = %issue.id, error = %e, "before_run hook failed"),
                    Err(_) => warn!(issue_id = %issue.id, "before_run hook timed out after 60s"),
                }
            }
        }

        // Read WORKFLOW.md from the workspace if it exists (offload blocking I/O).
        let workflow_file = self.workflow_file_for(&repo_name);
        let workflow_path = workspace.path.join(&workflow_file);
        let workflow_content = tokio::task::spawn_blocking(move || {
            std::fs::read_to_string(workflow_path).ok()
        })
        .await
        .expect("spawn_blocking panicked");
        info!(
            issue_id = %issue.id,
            has_workflow = workflow_content.is_some(),
            "workflow file check"
        );

        // Determine retry attempt from retry tracking.
        let attempt = self.retries.get(&issue.id).map(|r| r.attempt);

        // Build the agent task.
        let task = AgentTask {
            issue: issue.clone(),
            attempt,
            workflow_content,
        };

        // Start the agent.
        info!(issue_id = %issue.id, "starting agent process");
        let handle: AgentHandle = self.agent.start(&task, &workspace).await?;
        info!(issue_id = %issue.id, "agent process spawned successfully");

        // Take ownership of the child process for the watcher.
        let AgentHandle {
            child,
            started_at,
        } = handle;

        let watcher_issue_id = issue.id.clone();
        let ws_clone = workspace.clone();
        let queue = self.queue.clone();

        // Spawn a watcher task to monitor the child process.
        tokio::spawn(async move {
            let issue_id = watcher_issue_id;
            match child.wait_with_output().await {
                Ok(output) if output.status.success() => {
                    info!(issue_id = %issue_id, "agent process exited successfully");
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
                    warn!(issue_id = %issue_id, reason = %reason, "agent process failed");
                    queue.push(SymphonyEvent::AgentFailed {
                        issue_id,
                        workspace: ws_clone,
                        reason,
                    });
                }
                Err(e) => {
                    error!(issue_id = %issue_id, error = %e, "failed to wait on agent process");
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
        let log_issue_id = issue.id.clone();
        self.running.insert(
            issue.id.clone(),
            RunState {
                issue,
                workspace,
                started_at,
                last_activity: now,
            },
        );

        info!(issue_id = %log_issue_id, "dispatch complete, agent running");
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

        // Run after_run hook (with timeout).
        if let Some(rs) = &run_state {
            let repo_name = self.repo_name_for(&rs.issue.repo);
            if let Some(hooks) = self.workspace_mgr.hooks_for(&repo_name) {
                if let Some(script) = &hooks.after_run {
                    info!(issue_id = %issue_id, "running after_run hook");
                    let hook_timeout = Duration::from_secs(60);
                    match tokio::time::timeout(hook_timeout, run_hook(script, &workspace.path)).await {
                        Ok(Ok(())) => info!(issue_id = %issue_id, "after_run hook completed"),
                        Ok(Err(e)) => warn!(issue_id = %issue_id, error = %e, "after_run hook failed"),
                        Err(_) => warn!(issue_id = %issue_id, "after_run hook timed out after 60s"),
                    }
                }
            }

            // Cleanup the worktree (blocking git2 I/O — offload).
            let mgr = self.workspace_mgr.clone();
            let rn = repo_name.clone();
            let ws = workspace.clone();
            let cleanup_result = tokio::task::spawn_blocking(move || {
                mgr.cleanup_worktree(&rn, &ws)
            })
            .await
            .expect("spawn_blocking panicked");
            if let Err(e) = cleanup_result {
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

        let run_state = self.running.remove(issue_id);

        let entry = self
            .retries
            .entry(issue_id.to_owned())
            .and_modify(|e| e.attempt += 1)
            .or_insert_with(|| RetryEntry {
                attempt: 1,
                issue: run_state
                    .expect("run_state must exist for failed agent")
                    .issue,
            });
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
    async fn handle_state_changed(&mut self, issue_id: &str, new_state: &IssueState) -> Result<()> {
        info!(issue_id = %issue_id, state = ?new_state, "issue state changed");

        if *new_state == IssueState::Terminal {
            if let Some(run_state) = self.running.remove(issue_id) {
                self.claimed.remove(issue_id);
                self.retries.remove(issue_id);

                let repo_name = self.repo_name_for(&run_state.issue.repo);
                let mgr = self.workspace_mgr.clone();
                let rn = repo_name.clone();
                let ws = run_state.workspace.clone();
                let cleanup_result = tokio::task::spawn_blocking(move || {
                    mgr.cleanup_worktree(&rn, &ws)
                })
                .await
                .expect("spawn_blocking panicked");
                if let Err(e) = cleanup_result {
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

        self.claimed.remove(issue_id);
        self.running.remove(issue_id);

        let entry = match self.retries.get(issue_id) {
            Some(e) => e,
            None => {
                warn!(issue_id = %issue_id, "no retry entry found, dropping");
                return Ok(());
            }
        };
        let issue = entry.issue.clone();

        match self.tracker.fetch_issue_state(&issue).await {
            Ok(IssueState::Active) => {
                info!(issue_id = %issue_id, "issue still active, triggering re-poll");
                if let Err(e) = self.handle_poll_tick().await {
                    error!(error = %e, "re-poll after retry failed");
                }
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

    }

    // ── Status publishing ─────────────────────────────────────────────

    /// Publish a snapshot of the current orchestrator state to the status handle.
    async fn publish_snapshot(&self) {
        let now = Utc::now();

        let running: Vec<RunInfo> = self
            .running
            .iter()
            .map(|(id, rs)| {
                // Approximate DateTime<Utc> from Instant by computing elapsed.
                let elapsed = rs.started_at.elapsed();
                let started_at = now - chrono::Duration::from_std(elapsed).unwrap_or_default();

                RunInfo {
                    issue_id: id.clone(),
                    repo: rs.issue.repo.clone(),
                    title: rs.issue.title.clone(),
                    workspace_path: rs.workspace.path.display().to_string(),
                    branch: rs.workspace.branch.clone(),
                    started_at,
                }
            })
            .collect();

        let retries: Vec<RetryInfo> = self
            .retries
            .iter()
            .map(|(id, r)| RetryInfo {
                issue_id: id.clone(),
                attempt: r.attempt,
            })
            .collect();

        let snapshot = SymphonySnapshot {
            running,
            claimed: self.claimed.iter().cloned().collect(),
            retries,
            config_summary: ConfigSummary {
                enabled: self.config.enabled,
                poll_interval_secs: self.config.poll_interval.as_secs(),
                max_concurrent_agents: self.config.max_concurrent_agents,
                repos: self.config.repos.iter().map(|r| r.name.clone()).collect(),
            },
            updated_at: now,
        };

        self.status_handle.update_snapshot(snapshot).await;
    }

    /// Log an event to the broadcast channel.
    fn log_event(&self, kind: &str, issue_id: Option<&str>, detail: &str) {
        self.status_handle.log_event(SymphonyEventLog {
            timestamp: Utc::now(),
            kind: kind.to_string(),
            issue_id: issue_id.map(String::from),
            detail: detail.to_string(),
        });
    }

    /// Log a structured event from a `SymphonyEvent`.
    fn log_event_for(&self, event: &SymphonyEvent) {
        let (kind, issue_id, detail) = match event {
            SymphonyEvent::IssueDiscovered { issue } => {
                ("issue_discovered", Some(issue.id.as_str()), issue.title.as_str())
            }
            SymphonyEvent::AgentCompleted { issue_id, .. } => {
                ("agent_completed", Some(issue_id.as_str()), "")
            }
            SymphonyEvent::AgentFailed {
                issue_id, reason, ..
            } => ("agent_failed", Some(issue_id.as_str()), reason.as_str()),
            SymphonyEvent::AgentStalled { issue_id } => {
                ("agent_stalled", Some(issue_id.as_str()), "")
            }
            SymphonyEvent::RetryReady { issue_id } => {
                ("retry_ready", Some(issue_id.as_str()), "")
            }
            SymphonyEvent::IssueStateChanged {
                issue_id,
                new_state,
            } => {
                let detail_str = match new_state {
                    IssueState::Active => "active",
                    IssueState::Terminal => "terminal",
                };
                ("issue_state_changed", Some(issue_id.as_str()), detail_str)
            }
            SymphonyEvent::WorkspaceCleaned { issue_id, .. } => {
                ("workspace_cleaned", Some(issue_id.as_str()), "")
            }
            SymphonyEvent::Shutdown => ("shutdown", None, ""),
        };

        self.log_event(kind, issue_id, detail);
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

        async fn fetch_issue_state(&self, _issue: &TrackedIssue) -> Result<IssueState> {
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
            .poll_interval(Duration::from_millis(50))
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

        let status_handle = SymphonyStatusHandle::new(&config);
        let mut orchestrator = Orchestrator::new(
            Box::new(tracker),
            workspace_mgr,
            Box::new(agent),
            config,
            status_handle,
        );

        // Schedule a shutdown after the first poll tick fires.
        orchestrator
            .queue()
            .schedule_after(Duration::from_millis(150), SymphonyEvent::Shutdown);

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

        let status_handle = SymphonyStatusHandle::new(&config);
        let orchestrator = Orchestrator::new(
            Box::new(MockTracker {
                polled: Arc::new(AtomicBool::new(false)),
            }),
            WorkspaceManager::new(&[]),
            Box::new(MockAgent),
            config,
            status_handle,
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
