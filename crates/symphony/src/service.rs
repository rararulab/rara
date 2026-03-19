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

use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use chrono::Utc;
use snafu::ResultExt;
use tokio::{
    fs::OpenOptions,
    io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader},
    process::Child,
    sync::mpsc,
};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::{
    agent::{AgentTask, RalphAgent},
    config::{
        RepoConfig, SymphonyConfig, TrackerConfig, default_active_labels,
        default_repo_checkout_root, default_repo_url,
    },
    error::Result,
    tracker::{GitHubIssueTracker, IssueState, IssueTracker, LinearIssueTracker, TrackedIssue},
    workspace::{WorkspaceInfo, WorkspaceManager, workflow_file},
};

/// Tracks the state of a running ralph instance based on RPC events.
#[derive(Debug, Clone, Default)]
pub(crate) struct RunState {
    /// Current iteration number.
    pub(crate) iteration:       u32,
    /// Currently active hat name.
    pub(crate) current_hat:     String,
    /// Accumulated cost across all iterations.
    pub(crate) total_cost_usd:  f64,
    /// Whether the loop has terminated.
    pub(crate) terminated:      bool,
    /// Termination reason if terminated.
    pub(crate) term_reason:     Option<String>,
    /// Total RPC events received (replaces stdout_line_count for RPC mode).
    pub(crate) rpc_event_count: u64,
}

struct RunningIssue {
    issue:      TrackedIssue,
    workspace:  WorkspaceInfo,
    child:      Child,
    stdin:      Option<tokio::process::ChildStdin>,
    started_at: Instant,
    log_path:   PathBuf,
    output:     ProcessOutputSummaryHandle,
    rpc_rx:     mpsc::Receiver<crate::rpc::RpcEvent>,
    run_state:  RunState,
    attempt:    u32,
}

struct FinishedIssue {
    issue:     TrackedIssue,
    workspace: WorkspaceInfo,
    attempt:   u32,
    failed_at: Instant,
}

/// Top-level service that polls issue trackers, manages per-issue `ralph run`
/// subprocesses, and advances issue state in the external tracker.
pub struct SymphonyService {
    config:       SymphonyConfig,
    shutdown:     CancellationToken,
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

    pub async fn run(self) -> Result<()> {
        info!("starting symphony service");
        info!(lnav = %lnav_hint(), "ralpha issue logs are available");

        let tracker: Box<dyn IssueTracker> = self.build_tracker()?;
        let agent = RalphAgent::new(self.config.agent.clone());
        let mut runtime = IssueRuntime::new(self.config.clone(), agent);

        info!("symphony poll loop started");

        loop {
            tokio::select! {
                _ = self.shutdown.cancelled() => {
                    info!("symphony shutting down");
                    break;
                }
                _ = tokio::time::sleep(self.config.poll_interval) => {
                    runtime.poll_cycle(&*tracker).await;
                }
            }
        }

        runtime.shutdown().await;
        info!("symphony service stopped");
        Ok(())
    }

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
                ..
            }) => {
                let resolved_key = resolve_env_var(api_key)?;
                Ok(Box::new(LinearIssueTracker::new(
                    &resolved_key,
                    endpoint,
                    team_key.clone(),
                    project_slug.clone(),
                    active_states.clone(),
                    terminal_states.clone(),
                    repo_label_prefix.clone(),
                )?))
            }
            Some(TrackerConfig::Github { api_key, .. }) => {
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

struct IssueRuntime {
    config:            SymphonyConfig,
    workspace_manager: WorkspaceManager,
    agent:             RalphAgent,
    running:           HashMap<String, RunningIssue>,
    reviewing:         HashMap<String, RunningIssue>,
    failed:            HashMap<String, FinishedIssue>,
}

impl IssueRuntime {
    fn new(config: SymphonyConfig, agent: RalphAgent) -> Self {
        Self {
            config,
            workspace_manager: WorkspaceManager,
            agent,
            running: HashMap::new(),
            reviewing: HashMap::new(),
            failed: HashMap::new(),
        }
    }

    async fn poll_cycle(&mut self, tracker: &dyn IssueTracker) {
        self.reap_finished(tracker).await;

        let issues = match tracker.fetch_active_issues().await {
            Ok(issues) => issues,
            Err(err) => {
                warn!(error = ?err, "failed to fetch active issues");
                return;
            }
        };

        let active_ids: HashSet<String> = issues.iter().map(|issue| issue.id.clone()).collect();
        self.cleanup_terminal_issues(tracker, &active_ids).await;

        for issue in issues {
            let issue_id = issue.id.clone();
            let issue_identifier = issue.identifier.clone();
            let repo_name = issue.repo.clone();
            let issue_number = issue.number;

            if self.running.contains_key(&issue.id)
                || self.reviewing.contains_key(&issue.id)
                || self.failed.contains_key(&issue.id)
            {
                continue;
            }

            if self.running.len() >= self.config.max_concurrent_agents {
                info!(issue_id = %issue.id, "no global slot available");
                break;
            }

            if self
                .running
                .values()
                .filter(|run| run.issue.repo == issue.repo)
                .count()
                >= self.max_concurrent_for_repo(&issue.repo)
            {
                info!(issue_id = %issue.id, repo = %issue.repo, "no repo slot available");
                continue;
            }

            if let Err(err) = self.start_issue(tracker, issue, None).await {
                error!(
                    issue_id = %issue_id,
                    issue_identifier = %issue_identifier,
                    repo = %repo_name,
                    issue_number,
                    error = %err,
                    "failed to start issue run"
                );
            }
        }

        // Retry eligible failed issues after their backoff period
        self.retry_failed_issues(tracker).await;
    }

    async fn shutdown(&mut self) {
        for (issue_id, run) in &mut self.running {
            warn!(issue_id = %issue_id, "stopping active ralph run");
            let _ = run.child.kill().await;
        }
        self.running.clear();
        for (issue_id, run) in &mut self.reviewing {
            warn!(issue_id = %issue_id, "stopping active review");
            let _ = run.child.kill().await;
        }
        self.reviewing.clear();
    }

    async fn reap_finished(&mut self, tracker: &dyn IssueTracker) {
        // Drain available RPC events for all running issues.
        for run in self.running.values_mut() {
            while let Ok(event) = run.rpc_rx.try_recv() {
                run.run_state.rpc_event_count += 1;
                match &event {
                    crate::rpc::RpcEvent::IterationStart { iteration, hat, .. } => {
                        run.run_state.iteration = *iteration;
                        run.run_state.current_hat = hat.clone();
                    }
                    crate::rpc::RpcEvent::IterationEnd { cost_usd, .. } => {
                        run.run_state.total_cost_usd += cost_usd;
                    }
                    crate::rpc::RpcEvent::LoopTerminated { reason, .. } => {
                        run.run_state.terminated = true;
                        run.run_state.term_reason = Some(reason.clone());
                    }
                    _ => {}
                }
            }
        }

        let issue_ids: Vec<String> = self.running.keys().cloned().collect();
        let mut completed = Vec::new();
        let mut stalled = Vec::new();

        for issue_id in issue_ids {
            let Some(run) = self.running.get_mut(&issue_id) else {
                continue;
            };

            match run.child.try_wait() {
                Ok(Some(status)) => completed.push((issue_id, status)),
                Ok(None) => {
                    if run.started_at.elapsed() > self.config.stall_timeout {
                        stalled.push(issue_id);
                    }
                }
                Err(err) => {
                    warn!(issue_id = %issue_id, error = %err, "failed to poll ralph child status")
                }
            }
        }

        for (issue_id, status) in completed {
            let Some(run) = self.running.remove(&issue_id) else {
                continue;
            };
            let output = run.output.snapshot();

            if status.success() {
                info!(
                    issue_id = %issue_id,
                    elapsed_secs = run.started_at.elapsed().as_secs(),
                    log_path = %run.log_path.display(),
                    rpc_events = run.run_state.rpc_event_count,
                    stderr_lines = output.stderr_line_count,
                    "ralph coding run completed"
                );

                // If review is enabled and no review is already running, spawn
                // a reviewer before transitioning the issue to completed.
                if self.config.review.enabled && !self.reviewing.contains_key(&issue_id) {
                    match self
                        .start_review_for_issue(&run.issue, &run.workspace)
                        .await
                    {
                        Ok(()) => {
                            info!(issue_id = %issue_id, "started review phase");
                            continue;
                        }
                        Err(err) => {
                            warn!(
                                issue_id = %issue_id,
                                error = %err,
                                "failed to start review; falling through to completed"
                            );
                        }
                    }
                }

                // No review configured, or review failed to start — transition directly.
                let completed_state = self.completed_issue_state();
                if let Err(err) = tracker.transition_issue(&run.issue, completed_state).await {
                    warn!(issue_id = %issue_id, state = completed_state, error = %err, "failed to transition issue after successful ralph run");
                    self.failed.insert(
                        issue_id,
                        FinishedIssue {
                            issue:     run.issue,
                            workspace: run.workspace,
                            attempt:   run.attempt,
                            failed_at: Instant::now(),
                        },
                    );
                    continue;
                }
                self.cleanup_workspace(&run.issue.repo, &run.workspace);
            } else {
                warn!(
                    issue_id = %issue_id,
                    status = ?status.code(),
                    elapsed_secs = run.started_at.elapsed().as_secs(),
                    log_path = %run.log_path.display(),
                    rpc_events = run.run_state.rpc_event_count,
                    stderr_lines = output.stderr_line_count,
                    stderr_tail = %output.render_stderr_tail(),
                    "ralph task runner failed"
                );
                self.failed.insert(
                    issue_id,
                    FinishedIssue {
                        issue:     run.issue,
                        workspace: run.workspace,
                        attempt:   run.attempt,
                        failed_at: Instant::now(),
                    },
                );
            }
        }

        // Kill stalled processes that exceeded the configured timeout
        for issue_id in stalled {
            let Some(mut run) = self.running.remove(&issue_id) else {
                continue;
            };
            let elapsed = run.started_at.elapsed();
            warn!(
                issue_id = %issue_id,
                elapsed_secs = elapsed.as_secs(),
                stall_timeout_secs = self.config.stall_timeout.as_secs(),
                log_path = %run.log_path.display(),
                "killing stalled ralph agent"
            );
            let _ = run.child.kill().await;
            self.failed.insert(
                issue_id,
                FinishedIssue {
                    issue:     run.issue,
                    workspace: run.workspace,
                    attempt:   run.attempt,
                    failed_at: Instant::now(),
                },
            );
        }

        // Reap completed reviews.
        let review_ids: Vec<String> = self.reviewing.keys().cloned().collect();
        for issue_id in &review_ids {
            // Drain RPC events from the reviewer.
            if let Some(run) = self.reviewing.get_mut(issue_id) {
                while let Ok(event) = run.rpc_rx.try_recv() {
                    if let crate::rpc::RpcEvent::LoopTerminated { reason, .. } = &event {
                        run.run_state.terminated = true;
                        run.run_state.term_reason = Some(reason.clone());
                    }
                }
            }

            let should_reap = self
                .reviewing
                .get_mut(issue_id)
                .and_then(|run| run.child.try_wait().ok())
                .flatten();

            if let Some(status) = should_reap {
                let run = self.reviewing.remove(issue_id).expect("key just checked");
                if status.success() {
                    info!(issue_id = %issue_id, "review completed successfully");
                } else {
                    warn!(issue_id = %issue_id, status = ?status.code(), "review failed");
                }
                let completed_state = self.completed_issue_state();
                if let Err(err) = tracker.transition_issue(&run.issue, completed_state).await {
                    warn!(issue_id = %issue_id, error = %err, "failed to transition issue after review");
                }
                self.cleanup_workspace(&run.issue.repo, &run.workspace);
            } else if let Some(run) = self.reviewing.get(issue_id) {
                if run.started_at.elapsed() > self.config.stall_timeout {
                    let mut run = self.reviewing.remove(issue_id).expect("key just checked");
                    warn!(issue_id = %issue_id, "killing stalled reviewer");
                    let _ = run.child.kill().await;
                    self.cleanup_workspace(&run.issue.repo, &run.workspace);
                }
            }
        }
    }

    async fn cleanup_terminal_issues(
        &mut self,
        tracker: &dyn IssueTracker,
        active_ids: &HashSet<String>,
    ) {
        let known_ids: Vec<String> = self
            .running
            .keys()
            .chain(self.reviewing.keys())
            .chain(self.failed.keys())
            .cloned()
            .collect();

        for issue_id in known_ids {
            if active_ids.contains(&issue_id) {
                continue;
            }

            let issue = self
                .running
                .get(&issue_id)
                .map(|run| run.issue.clone())
                .or_else(|| self.reviewing.get(&issue_id).map(|run| run.issue.clone()))
                .or_else(|| self.failed.get(&issue_id).map(|run| run.issue.clone()));
            let Some(issue) = issue else {
                continue;
            };

            let state = match tracker.fetch_issue_state(&issue).await {
                Ok(state) => state,
                Err(err) => {
                    warn!(issue_id = %issue_id, error = %err, "failed to refresh issue state");
                    continue;
                }
            };

            if state != IssueState::Terminal {
                continue;
            }

            if let Some(mut run) = self.running.remove(&issue_id) {
                let _ = run.child.kill().await;
                self.cleanup_workspace(&run.issue.repo, &run.workspace);
            }
            if let Some(mut run) = self.reviewing.remove(&issue_id) {
                let _ = run.child.kill().await;
                self.cleanup_workspace(&run.issue.repo, &run.workspace);
            }
            if let Some(run) = self.failed.remove(&issue_id) {
                self.cleanup_workspace(&run.issue.repo, &run.workspace);
            }
        }
    }

    /// Exponential backoff: min(2^attempt * 60s, max_retry_backoff).
    fn retry_delay(&self, attempt: u32) -> Duration {
        let base = Duration::from_secs(60);
        let shift = attempt.min(31);
        let exp = base.saturating_mul(1u32 << shift);
        exp.min(self.config.max_retry_backoff)
    }

    /// Re-dispatch failed issues whose backoff period has elapsed.
    async fn retry_failed_issues(&mut self, tracker: &dyn IssueTracker) {
        let eligible: Vec<String> = self
            .failed
            .iter()
            .filter(|(_, finished)| {
                let delay = self.retry_delay(finished.attempt);
                finished.failed_at.elapsed() >= delay
            })
            .map(|(id, _)| id.clone())
            .collect();

        for issue_id in eligible {
            if self.running.len() >= self.config.max_concurrent_agents {
                break;
            }

            let Some(finished) = self.failed.remove(&issue_id) else {
                continue;
            };

            let next_attempt = finished.attempt + 1;
            info!(
                issue_id = %issue_id,
                attempt = next_attempt,
                "retrying failed issue"
            );

            let issue = finished.issue;
            match self
                .start_issue(tracker, issue.clone(), Some(next_attempt))
                .await
            {
                Ok(()) => {
                    // Clean up old workspace only after successful re-provisioning
                    self.cleanup_workspace(&issue.repo, &finished.workspace);
                }
                Err(err) => {
                    error!(
                        issue_id = %issue_id,
                        attempt = next_attempt,
                        error = %err,
                        "failed to retry issue"
                    );
                    self.failed.insert(
                        issue_id,
                        FinishedIssue {
                            issue,
                            workspace: finished.workspace,
                            attempt: next_attempt,
                            failed_at: Instant::now(),
                        },
                    );
                }
            }
        }
    }

    /// Spawn a ralph reviewer for the given issue, reusing the existing
    /// workspace.
    async fn start_review_for_issue(
        &mut self,
        issue: &TrackedIssue,
        workspace: &WorkspaceInfo,
    ) -> Result<()> {
        let mut handle = self
            .agent
            .start_review(issue, &workspace.path, &self.config.review)
            .await?;

        let log_path = issue_log_path(&issue.repo, &format!("{}-review", issue.identifier));
        let log_writer = spawn_issue_log_writer(&log_path, issue, workspace).await?;
        let output = ProcessOutputSummaryHandle::default();
        let (rpc_tx, rpc_rx) = mpsc::channel(256);

        if let Some(stdout) = handle.child.stdout.take() {
            let lw = log_writer.clone();
            let (raw_tx, mut raw_rx) = mpsc::channel::<String>(256);
            tokio::spawn(async move {
                while let Some(line) = raw_rx.recv().await {
                    let _ = lw.record("stdout", &line).await;
                }
            });
            let _rpc_reader = crate::rpc_reader::spawn_rpc_reader(rpc_tx, raw_tx, stdout);
        }
        if let Some(stderr) = handle.child.stderr.take() {
            spawn_stream_logger(output.clone(), log_writer, "stderr", stderr);
        }

        info!(
            issue_id = %issue.id,
            issue_identifier = %issue.identifier,
            log_path = %log_path.display(),
            "spawned ralph reviewer"
        );

        self.reviewing.insert(
            issue.id.clone(),
            RunningIssue {
                issue: issue.clone(),
                workspace: workspace.clone(),
                child: handle.child,
                stdin: handle.stdin,
                started_at: handle.started_at,
                log_path,
                output,
                rpc_rx,
                run_state: RunState::default(),
                attempt: 0,
            },
        );
        Ok(())
    }

    /// Provision a worktree, start `ralph run`, attach raw output logging, and
    /// transition the issue to `In Progress` once the child is live.
    async fn start_issue(
        &mut self,
        tracker: &dyn IssueTracker,
        issue: TrackedIssue,
        attempt: Option<u32>,
    ) -> Result<()> {
        let repo = self.repo_config(&issue.repo).with_context(|_| {
            crate::error::WorkspaceContextSnafu {
                message: format!(
                    "failed to resolve repo config for issue {} ({}) in repo {}",
                    issue.identifier, issue.id, issue.repo
                ),
            }
        })?;
        let workspace = self
            .workspace_manager
            .ensure_worktree(&repo, issue.number, &issue.title)
            .with_context(|_| crate::error::WorkspaceContextSnafu {
                message: format!(
                    "failed to ensure worktree for issue {} ({}) in repo {} under {}",
                    issue.identifier,
                    issue.id,
                    issue.repo,
                    repo.effective_workspace_root()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| String::from("<unset>"))
                ),
            })?;
        let workflow_path = workspace
            .path
            .join(workflow_file(&repo, &self.config.workflow_file));
        let workflow_content = match tokio::fs::read_to_string(&workflow_path).await {
            Ok(content) => Some(content),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => None,
            Err(source) => {
                warn!(
                    issue_id = %issue.id,
                    issue_identifier = %issue.identifier,
                    repo = %issue.repo,
                    workflow_path = %workflow_path.display(),
                    error = %source,
                    "failed to read workflow file; continuing with synthesized prompt"
                );
                None
            }
        };

        let task = AgentTask {
            issue: issue.clone(),
            attempt,
            workflow_content,
        };
        let mut handle = self
            .agent
            .start(&task, &workspace.path)
            .await
            .with_context(|_| crate::error::WorkspaceContextSnafu {
                message: format!(
                    "failed to start agent for issue {} ({}) in repo {} at workspace {}",
                    issue.identifier,
                    issue.id,
                    issue.repo,
                    workspace.path.display()
                ),
            })?;
        let log_path = issue_log_path(&issue.repo, &issue.identifier);
        let log_writer = spawn_issue_log_writer(&log_path, &issue, &workspace)
            .await
            .with_context(|_| crate::error::WorkspaceContextSnafu {
                message: format!(
                    "failed to create issue log writer for issue {} ({}) in repo {} at {}",
                    issue.identifier,
                    issue.id,
                    issue.repo,
                    log_path.display()
                ),
            })?;
        let output = ProcessOutputSummaryHandle::default();

        info!(
            issue_id = %issue.id,
            repo = %issue.repo,
            workspace = %workspace.path.display(),
            branch = %workspace.branch,
            log_path = %log_path.display(),
            created_now = workspace.created_now,
            "spawned ralph task runner"
        );

        let (rpc_tx, rpc_rx) = mpsc::channel::<crate::rpc::RpcEvent>(256);

        if let Some(stdout) = handle.child.stdout.take() {
            // Forward unparseable lines to the log file as fallback.
            let lw = log_writer.clone();
            let (raw_tx, mut raw_rx) = mpsc::channel::<String>(256);
            tokio::spawn(async move {
                while let Some(line) = raw_rx.recv().await {
                    let _ = lw.record("stdout", &line).await;
                }
            });
            let _rpc_reader = crate::rpc_reader::spawn_rpc_reader(rpc_tx, raw_tx, stdout);
        }
        if let Some(stderr) = handle.child.stderr.take() {
            spawn_stream_logger(output.clone(), log_writer, "stderr", stderr);
        }

        let started_state = self.started_issue_state();
        if let Err(err) = tracker.transition_issue(&issue, started_state).await {
            warn!(
                issue_id = %issue.id,
                state = started_state,
                error = %err,
                "failed to transition issue after starting ralph task runner"
            );
        }

        self.running.insert(
            issue.id.clone(),
            RunningIssue {
                issue,
                workspace,
                child: handle.child,
                stdin: handle.stdin,
                started_at: handle.started_at,
                log_path,
                output,
                rpc_rx,
                run_state: RunState::default(),
                attempt: attempt.unwrap_or(0),
            },
        );
        Ok(())
    }

    fn cleanup_workspace(&self, repo_name: &str, workspace: &WorkspaceInfo) {
        match self.repo_config(repo_name) {
            Ok(repo) => {
                if let Err(err) = self.workspace_manager.cleanup_worktree(&repo, workspace) {
                    warn!(repo = %repo_name, path = %workspace.path.display(), error = %err, "failed to cleanup workspace");
                }
            }
            Err(err) => {
                warn!(repo = %repo_name, error = %err, "failed to resolve repo for workspace cleanup")
            }
        }
    }

    fn repo_config(&self, repo_name: &str) -> Result<RepoConfig> {
        let repo = self
            .config
            .repos
            .iter()
            .find(|repo| repo.name == repo_name)
            .cloned()
            .unwrap_or_else(|| self.derived_repo_config(repo_name));

        let mut resolved = repo;
        if resolved.repo_path.is_none() {
            let cwd = std::env::current_dir().context(crate::error::IoSnafu)?;
            resolved.repo_path = Some(cwd.clone());
        }
        Ok(resolved)
    }

    fn derived_repo_config(&self, repo_name: &str) -> RepoConfig {
        RepoConfig::builder()
            .name(repo_name.to_owned())
            .url(default_repo_url(repo_name))
            .repo_path(default_repo_checkout_root(repo_name))
            .active_labels(default_active_labels())
            .build()
    }

    fn max_concurrent_for_repo(&self, repo_name: &str) -> usize {
        self.config
            .repos
            .iter()
            .find(|repo| repo.name == repo_name)
            .and_then(|repo| repo.max_concurrent_agents)
            .unwrap_or(self.config.max_concurrent_agents)
    }

    fn started_issue_state(&self) -> &str {
        self.config
            .tracker
            .as_ref()
            .map_or("In Progress", TrackerConfig::started_issue_state)
    }

    fn completed_issue_state(&self) -> &str {
        self.config
            .tracker
            .as_ref()
            .map_or("ToVerify", TrackerConfig::completed_issue_state)
    }

    /// Send an RPC command to the ralph process for the given issue.
    async fn send_command(&mut self, issue_id: &str, cmd: crate::rpc::RpcCommand) -> Result<()> {
        let run = self.running.get_mut(issue_id).ok_or_else(|| {
            crate::error::RpcSnafu {
                message: format!("no running agent for issue {issue_id}"),
            }
            .build()
        })?;

        let stdin = run.stdin.as_mut().ok_or_else(|| {
            crate::error::RpcSnafu {
                message: format!("stdin not available for issue {issue_id}"),
            }
            .build()
        })?;

        let mut line = serde_json::to_string(&cmd).map_err(|e| {
            crate::error::RpcSnafu {
                message: format!("failed to serialize RPC command: {e}"),
            }
            .build()
        })?;
        line.push('\n');

        stdin
            .write_all(line.as_bytes())
            .await
            .context(crate::error::RpcIoSnafu {
                message: format!("failed to write RPC command to ralph for issue {issue_id}"),
            })?;

        info!(issue_id, command = ?cmd, "sent RPC command to ralph");
        Ok(())
    }

    /// Send guidance to ralph for the next iteration.
    pub(crate) async fn send_guidance(&mut self, issue_id: &str, message: String) -> Result<()> {
        self.send_command(
            issue_id,
            crate::rpc::RpcCommand::Guidance { id: None, message },
        )
        .await
    }

    /// Steer ralph immediately in the current iteration.
    pub(crate) async fn steer(&mut self, issue_id: &str, message: String) -> Result<()> {
        self.send_command(
            issue_id,
            crate::rpc::RpcCommand::Steer { id: None, message },
        )
        .await
    }

    /// Gracefully abort ralph for the given issue.
    pub(crate) async fn abort(&mut self, issue_id: &str, reason: Option<String>) -> Result<()> {
        self.send_command(issue_id, crate::rpc::RpcCommand::Abort { id: None, reason })
            .await
    }

    /// Query the current run state of a running ralph instance.
    pub(crate) fn run_state(&self, issue_id: &str) -> Option<&RunState> {
        self.running.get(issue_id).map(|run| &run.run_state)
    }
}

fn spawn_stream_logger<R: AsyncRead + Unpin + Send + 'static>(
    output: ProcessOutputSummaryHandle,
    log_writer: IssueLogWriter,
    stream_name: &'static str,
    reader: R,
) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            output.record(stream_name, line.clone());
            let _ = log_writer.record(stream_name, &line).await;
        }
    });
}

/// Async file-backed sink for raw `ralph` stdout/stderr lines.
#[derive(Clone, Debug)]
struct IssueLogWriter {
    sender: mpsc::Sender<String>,
}

impl IssueLogWriter {
    async fn record(&self, stream_name: &'static str, line: &str) -> Result<()> {
        let entry = format!("{} [{}] {}\n", Utc::now().to_rfc3339(), stream_name, line);
        self.sender.send(entry).await.map_err(|_| {
            crate::error::WorkspaceSnafu {
                message: String::from("issue log writer closed unexpectedly"),
            }
            .build()
        })
    }
}

/// Create the per-issue log file and spawn a background append loop.
async fn spawn_issue_log_writer(
    log_path: &Path,
    issue: &TrackedIssue,
    workspace: &WorkspaceInfo,
) -> Result<IssueLogWriter> {
    let parent = log_path.parent().ok_or_else(|| {
        crate::error::WorkspaceSnafu {
            message: format!("issue log path has no parent: {}", log_path.display()),
        }
        .build()
    })?;
    tokio::fs::create_dir_all(parent)
        .await
        .context(crate::error::WorkspaceIoSnafu {
            message: format!(
                "failed to create issue log directory {} for issue {} in repo {}",
                parent.display(),
                issue.identifier,
                issue.repo
            ),
        })?;

    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(log_path)
        .await
        .context(crate::error::WorkspaceIoSnafu {
            message: format!(
                "failed to open issue log file {} for issue {} in repo {}",
                log_path.display(),
                issue.identifier,
                issue.repo
            ),
        })?;
    let header = format!(
        "{} [meta] issue={} repo={} branch={} workspace={}\n",
        Utc::now().to_rfc3339(),
        issue.identifier,
        issue.repo,
        workspace.branch,
        workspace.path.display(),
    );
    file.write_all(header.as_bytes())
        .await
        .context(crate::error::WorkspaceIoSnafu {
            message: format!(
                "failed to write issue log header to {} for issue {} in repo {}",
                log_path.display(),
                issue.identifier,
                issue.repo
            ),
        })?;

    let (sender, mut receiver) = mpsc::channel::<String>(256);
    tokio::spawn(async move {
        while let Some(entry) = receiver.recv().await {
            if file.write_all(entry.as_bytes()).await.is_err() {
                return;
            }
        }
    });

    Ok(IssueLogWriter { sender })
}

/// Log file layout: `~/.config/rara/ralpha/logs/<repo>/<ISSUE>.log`.
fn issue_log_path(repo_name: &str, issue_identifier: &str) -> PathBuf {
    rara_paths::config_dir()
        .join("ralpha/logs")
        .join(repo_name)
        .join(format!("{issue_identifier}.log"))
}

/// Shell hint printed at startup so operators can inspect issue logs quickly.
fn lnav_hint() -> String {
    format!(
        "lnav {}/**/*.log",
        rara_paths::config_dir().join("ralpha/logs").display()
    )
}

#[derive(Debug, Clone, Default)]
struct ProcessOutputSummaryHandle(std::sync::Arc<std::sync::Mutex<ProcessOutputSummary>>);

impl ProcessOutputSummaryHandle {
    fn record(&self, stream_name: &'static str, line: String) {
        match self.0.lock() {
            Ok(mut guard) => guard.record(stream_name, line),
            Err(_) => warn!("ProcessOutputSummary mutex poisoned, dropping output line"),
        }
    }

    fn snapshot(&self) -> ProcessOutputSummary {
        match self.0.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => {
                warn!("ProcessOutputSummary mutex poisoned, recovering");
                poisoned.into_inner().clone()
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
struct ProcessOutputSummary {
    stdout_line_count: usize,
    stderr_line_count: usize,
    stderr_tail:       VecDeque<String>,
}

impl ProcessOutputSummary {
    fn record(&mut self, stream_name: &'static str, line: String) {
        match stream_name {
            "stdout" => {
                self.stdout_line_count += 1;
            }
            "stderr" => {
                self.stderr_line_count += 1;
                if !line.trim().is_empty() {
                    if self.stderr_tail.len() == 6 {
                        self.stderr_tail.pop_front();
                    }
                    self.stderr_tail.push_back(line);
                }
            }
            _ => {}
        }
    }

    fn render_stderr_tail(&self) -> String {
        if self.stderr_tail.is_empty() {
            String::from("<none>")
        } else {
            self.stderr_tail
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(" | ")
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

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use tokio_util::sync::CancellationToken;

    use super::{IssueRuntime, ProcessOutputSummary, SymphonyService, issue_log_path, lnav_hint};
    use crate::config::{AgentConfig, RepoConfig, ReviewConfig, SymphonyConfig};

    #[test]
    fn process_output_summary_keeps_only_recent_stderr_lines() {
        let mut summary = ProcessOutputSummary::default();
        summary.record("stdout", "boot".to_owned());
        for i in 0..8 {
            summary.record("stderr", format!("line-{i}"));
        }

        assert_eq!(summary.stdout_line_count, 1);
        assert_eq!(summary.stderr_line_count, 8);
        assert_eq!(
            summary.render_stderr_tail(),
            "line-2 | line-3 | line-4 | line-5 | line-6 | line-7"
        );
    }

    #[test]
    fn issue_log_path_is_scoped_per_repo_and_issue() {
        assert_eq!(
            issue_log_path("rararulab/rara", "RAR-123"),
            rara_paths::config_dir()
                .join("ralpha/logs")
                .join("rararulab/rara")
                .join("RAR-123.log")
        );
    }

    #[test]
    fn lnav_hint_points_at_ralpha_logs_dir() {
        let hint = lnav_hint();
        assert!(hint.contains("lnav"));
        assert!(hint.contains("/ralpha/logs"));
    }

    #[test]
    fn repo_config_derives_unknown_repo() {
        let service = SymphonyService::new(
            SymphonyConfig::builder()
                .enabled(true)
                .poll_interval(Duration::from_secs(30))
                .max_concurrent_agents(2)
                .stall_timeout(Duration::from_secs(30 * 60))
                .max_retry_backoff(Duration::from_secs(60 * 60))
                .workflow_file("WORKFLOW.md".to_owned())
                .agent(AgentConfig::default())
                .review(ReviewConfig::default())
                .repos(vec![])
                .build(),
            CancellationToken::new(),
            None,
        );

        let repo = IssueRuntime::new(
            service.config.clone(),
            crate::agent::RalphAgent::new(AgentConfig::default()),
        )
        .repo_config("crrowbot/rara-notes")
        .expect("fallback repo config should resolve");

        assert_eq!(repo.name, "crrowbot/rara-notes");
        assert_eq!(repo.url, "git@github.com:crrowbot/rara-notes.git");
        assert_eq!(
            repo.repo_path,
            Some(
                rara_paths::config_dir()
                    .join("ralpha/repos")
                    .join("crrowbot/rara-notes")
            )
        );
    }

    #[test]
    fn repo_config_prefers_explicit_repo_settings() {
        let configured = RepoConfig::builder()
            .name("rararulab/rara".to_owned())
            .url("https://example.com/custom.git".to_owned())
            .active_labels(vec!["symphony:ready".to_owned()])
            .build();
        let runtime = IssueRuntime::new(
            SymphonyConfig::builder()
                .enabled(true)
                .poll_interval(Duration::from_secs(30))
                .max_concurrent_agents(2)
                .stall_timeout(Duration::from_secs(30 * 60))
                .max_retry_backoff(Duration::from_secs(60 * 60))
                .workflow_file("WORKFLOW.md".to_owned())
                .agent(AgentConfig::default())
                .review(ReviewConfig::default())
                .repos(vec![configured])
                .build(),
            crate::agent::RalphAgent::new(AgentConfig::default()),
        );

        let repo = runtime
            .repo_config("rararulab/rara")
            .expect("configured repo should resolve");

        assert_eq!(repo.url, "https://example.com/custom.git");
    }

    #[test]
    fn retry_delay_grows_exponentially_up_to_max() {
        let runtime = IssueRuntime::new(
            SymphonyConfig::builder()
                .enabled(true)
                .poll_interval(Duration::from_secs(30))
                .max_concurrent_agents(2)
                .stall_timeout(Duration::from_secs(30 * 60))
                .max_retry_backoff(Duration::from_secs(600))
                .workflow_file("WORKFLOW.md".to_owned())
                .agent(AgentConfig::default())
                .review(ReviewConfig::default())
                .repos(vec![])
                .build(),
            crate::agent::RalphAgent::new(AgentConfig::default()),
        );

        assert_eq!(runtime.retry_delay(0), Duration::from_secs(60));
        assert_eq!(runtime.retry_delay(1), Duration::from_secs(120));
        assert_eq!(runtime.retry_delay(2), Duration::from_secs(240));
        assert_eq!(runtime.retry_delay(3), Duration::from_secs(480));
        assert_eq!(runtime.retry_delay(4), Duration::from_secs(600)); // capped
    }

    #[test]
    fn stalled_issue_is_detected_by_elapsed_time() {
        let timeout = Duration::from_secs(10);
        let started = Instant::now()
            .checked_sub(Duration::from_secs(20))
            .expect("20s subtraction should not underflow");
        assert!(started.elapsed() > timeout);

        let started_recent = Instant::now()
            .checked_sub(Duration::from_secs(5))
            .expect("5s subtraction should not underflow");
        assert!(started_recent.elapsed() <= timeout);
    }
}
