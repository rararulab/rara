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
    time::Instant,
};

use chrono::Utc;
use snafu::ResultExt;
use tokio::{
    fs::OpenOptions,
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStderr, ChildStdout},
    sync::{Mutex, mpsc},
};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::{
    agent::{AgentTask, RalphAgent},
    config::{
        RepoConfig, SymphonyConfig, TrackerConfig, default_active_labels,
        default_repo_checkout_root, default_repo_url,
    },
    error::{IoSnafu, Result},
    tracker::{GitHubIssueTracker, IssueState, IssueTracker, LinearIssueTracker, TrackedIssue},
    workspace::{WorkspaceInfo, WorkspaceManager, workflow_file},
};

struct RunningIssue {
    issue: TrackedIssue,
    workspace: WorkspaceInfo,
    child: Child,
    started_at: Instant,
    log_path: PathBuf,
    output: ProcessOutputSummaryHandle,
}

struct FinishedIssue {
    issue: TrackedIssue,
    workspace: WorkspaceInfo,
}

/// Top-level service that polls issue trackers, manages per-issue `ralph run`
/// subprocesses, and advances issue state in the external tracker.
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
    config: SymphonyConfig,
    workspace_manager: WorkspaceManager,
    agent: RalphAgent,
    running: HashMap<String, RunningIssue>,
    failed: HashMap<String, FinishedIssue>,
}

impl IssueRuntime {
    fn new(config: SymphonyConfig, agent: RalphAgent) -> Self {
        Self {
            config,
            workspace_manager: WorkspaceManager,
            agent,
            running: HashMap::new(),
            failed: HashMap::new(),
        }
    }

    async fn poll_cycle(&mut self, tracker: &dyn IssueTracker) {
        self.reap_finished(tracker).await;

        let issues = match tracker.fetch_active_issues().await {
            Ok(issues) => issues,
            Err(err) => {
                warn!(error = %err, "failed to fetch active issues");
                return;
            }
        };

        let active_ids: HashSet<String> = issues.iter().map(|issue| issue.id.clone()).collect();
        self.cleanup_terminal_issues(tracker, &active_ids).await;

        for issue in issues {
            if self.running.contains_key(&issue.id) || self.failed.contains_key(&issue.id) {
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

            if let Err(err) = self.start_issue(tracker, issue).await {
                error!(error = %err, "failed to start issue run");
            }
        }
    }

    async fn shutdown(&mut self) {
        for (issue_id, run) in &mut self.running {
            warn!(issue_id = %issue_id, "stopping active ralph run");
            let _ = run.child.kill().await;
        }
        self.running.clear();
    }

    async fn reap_finished(&mut self, tracker: &dyn IssueTracker) {
        let issue_ids: Vec<String> = self.running.keys().cloned().collect();
        let mut completed = Vec::new();

        for issue_id in issue_ids {
            let Some(run) = self.running.get_mut(&issue_id) else {
                continue;
            };

            match run.child.try_wait() {
                Ok(Some(status)) => completed.push((issue_id, status)),
                Ok(None) => {}
                Err(err) => {
                    warn!(issue_id = %issue_id, error = %err, "failed to poll ralph child status")
                }
            }
        }

        for (issue_id, status) in completed {
            let Some(run) = self.running.remove(&issue_id) else {
                continue;
            };
            let output = run.output.snapshot().await;

            if status.success() {
                info!(
                    issue_id = %issue_id,
                    elapsed_secs = run.started_at.elapsed().as_secs(),
                    log_path = %run.log_path.display(),
                    stdout_lines = output.stdout_line_count,
                    stderr_lines = output.stderr_line_count,
                    "ralph task runner completed"
                );
                let completed_state = self.completed_issue_state();
                if let Err(err) = tracker.transition_issue(&run.issue, completed_state).await {
                    warn!(issue_id = %issue_id, state = completed_state, error = %err, "failed to transition issue after successful ralph run");
                    self.failed.insert(
                        issue_id,
                        FinishedIssue {
                            issue: run.issue,
                            workspace: run.workspace,
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
                    stdout_lines = output.stdout_line_count,
                    stderr_lines = output.stderr_line_count,
                    stderr_tail = %output.render_stderr_tail(),
                    "ralph task runner failed"
                );
                self.failed.insert(
                    issue_id,
                    FinishedIssue {
                        issue: run.issue,
                        workspace: run.workspace,
                    },
                );
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
            if let Some(run) = self.failed.remove(&issue_id) {
                self.cleanup_workspace(&run.issue.repo, &run.workspace);
            }
        }
    }

    /// Provision a worktree, start `ralph run`, attach raw output logging, and
    /// transition the issue to `In Progress` once the child is live.
    async fn start_issue(&mut self, tracker: &dyn IssueTracker, issue: TrackedIssue) -> Result<()> {
        let repo = self.repo_config(&issue.repo)?;
        let workspace =
            self.workspace_manager
                .ensure_worktree(&repo, issue.number, &issue.title)?;
        let workflow_path = workspace
            .path
            .join(workflow_file(&repo, &self.config.workflow_file));
        let workflow_content = tokio::fs::read_to_string(&workflow_path).await.ok();

        let task = AgentTask {
            issue: issue.clone(),
            attempt: None,
            workflow_content,
        };
        let mut handle = self.agent.start(&task, &workspace.path).await?;
        let log_path = issue_log_path(&issue.repo, &issue.identifier);
        let log_writer = spawn_issue_log_writer(&log_path, &issue, &workspace).await?;
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

        if let Some(stdout) = handle.child.stdout.take() {
            spawn_output_logger(output.clone(), log_writer.clone(), "stdout", stdout);
        }
        if let Some(stderr) = handle.child.stderr.take() {
            spawn_error_logger(output.clone(), log_writer, "stderr", stderr);
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
                started_at: handle.started_at,
                log_path,
                output,
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
            let cwd =
                std::env::current_dir().map_err(|source| crate::error::SymphonyError::Io {
                    source,
                    location: snafu::Location::new(file!(), line!(), column!()),
                })?;
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
}

fn spawn_output_logger(
    output: ProcessOutputSummaryHandle,
    log_writer: IssueLogWriter,
    stream_name: &'static str,
    stdout: ChildStdout,
) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            output.record(stream_name, line.clone()).await;
            let _ = log_writer.record(stream_name, &line).await;
        }
    });
}

fn spawn_error_logger(
    output: ProcessOutputSummaryHandle,
    log_writer: IssueLogWriter,
    stream_name: &'static str,
    stderr: ChildStderr,
) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            output.record(stream_name, line.clone()).await;
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
        self.sender
            .send(entry)
            .await
            .map_err(|_| crate::error::SymphonyError::Workspace {
                message: String::from("issue log writer closed unexpectedly"),
                location: snafu::Location::new(file!(), line!(), column!()),
            })
    }
}

/// Create the per-issue log file and spawn a background append loop.
async fn spawn_issue_log_writer(
    log_path: &Path,
    issue: &TrackedIssue,
    workspace: &WorkspaceInfo,
) -> Result<IssueLogWriter> {
    let parent = log_path
        .parent()
        .ok_or_else(|| crate::error::SymphonyError::Workspace {
            message: format!("issue log path has no parent: {}", log_path.display()),
            location: snafu::Location::new(file!(), line!(), column!()),
        })?;
    tokio::fs::create_dir_all(parent).await.context(IoSnafu)?;

    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(log_path)
        .await
        .context(IoSnafu)?;
    let header = format!(
        "{} [meta] issue={} repo={} branch={} workspace={}\n",
        Utc::now().to_rfc3339(),
        issue.identifier,
        issue.repo,
        workspace.branch,
        workspace.path.display(),
    );
    file.write_all(header.as_bytes()).await.context(IoSnafu)?;

    let (sender, mut receiver) = mpsc::channel::<String>(256);
    let log_path = log_path.to_path_buf();
    tokio::spawn(async move {
        let open_result = OpenOptions::new().append(true).open(&log_path).await;
        let Ok(mut file) = open_result else {
            return;
        };

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
struct ProcessOutputSummaryHandle(std::sync::Arc<Mutex<ProcessOutputSummary>>);

impl ProcessOutputSummaryHandle {
    async fn record(&self, stream_name: &'static str, line: String) {
        self.0.lock().await.record(stream_name, line);
    }

    async fn snapshot(&self) -> ProcessOutputSummary {
        self.0.lock().await.clone()
    }
}

#[derive(Debug, Clone, Default)]
struct ProcessOutputSummary {
    stdout_line_count: usize,
    stderr_line_count: usize,
    stderr_tail: VecDeque<String>,
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
    use std::time::Duration;

    use tokio_util::sync::CancellationToken;

    use super::{IssueRuntime, ProcessOutputSummary, SymphonyService, issue_log_path, lnav_hint};
    use crate::config::{AgentConfig, RepoConfig, SymphonyConfig};

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
        assert_eq!(repo.url, "https://github.com/crrowbot/rara-notes");
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
                .repos(vec![configured])
                .build(),
            crate::agent::RalphAgent::new(AgentConfig::default()),
        );

        let repo = runtime
            .repo_config("rararulab/rara")
            .expect("configured repo should resolve");

        assert_eq!(repo.url, "https://example.com/custom.git");
    }
}
