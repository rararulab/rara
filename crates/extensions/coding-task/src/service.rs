// Copyright 2025 Crrow
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

//! Coding task orchestration service.

use std::sync::Arc;

use rara_domain_shared::{
    notify::{
        client::NotifyClient,
        types::{NotificationPriority, SendTelegramNotificationRequest},
    },
    settings::SettingsSvc,
};
use rara_workspace::WorkspaceManager;
use tokio::process::Command;
use tracing::{info, warn};
use uuid::Uuid;

use crate::error::{CodingTaskError, ExecutionSnafu, WorkspaceSnafu};
use crate::repository::CodingTaskRepository;
use crate::types::{AgentType, CodingTask, CodingTaskStatus};

/// Orchestrates coding task lifecycle: workspace setup, agent dispatch,
/// PR creation, notifications.
#[derive(Clone)]
pub struct CodingTaskService {
    repo:              Arc<dyn CodingTaskRepository>,
    workspace_manager: WorkspaceManager,
    notify:            NotifyClient,
    settings_svc:      SettingsSvc,
    default_repo_url:  String,
}

impl CodingTaskService {
    pub fn new(
        repo: Arc<dyn CodingTaskRepository>,
        workspace_manager: WorkspaceManager,
        notify: NotifyClient,
        settings_svc: SettingsSvc,
        default_repo_url: String,
    ) -> Self {
        Self {
            repo,
            workspace_manager,
            notify,
            settings_svc,
            default_repo_url,
        }
    }

    /// Dispatch a new coding task.
    ///
    /// Creates a DB record, prepares workspace & worktree, then spawns the
    /// agent in a tmux session. Returns the task immediately.
    pub async fn dispatch(
        &self,
        repo_url: Option<&str>,
        prompt: &str,
        agent_type: AgentType,
        session_key: Option<String>,
    ) -> Result<CodingTask, CodingTaskError> {
        let repo_url = repo_url.unwrap_or(&self.default_repo_url);
        let task_id = Uuid::new_v4();
        let short_id = &task_id.to_string()[..8];
        let branch = format!("rara/task-{short_id}");
        let tmux_session = format!("rara-{short_id}");

        let task = CodingTask {
            id: task_id,
            status: CodingTaskStatus::Pending,
            agent_type,
            repo_url: repo_url.to_owned(),
            branch: branch.clone(),
            prompt: prompt.to_owned(),
            pr_url: None,
            pr_number: None,
            session_key,
            tmux_session: tmux_session.clone(),
            workspace_path: String::new(),
            output: String::new(),
            exit_code: None,
            error: None,
            created_at: jiff::Timestamp::now(),
            started_at: None,
            completed_at: None,
        };

        let task = self.repo.create(&task).await?;
        info!(
            id = %task.id, branch = %branch, agent = %agent_type,
            "dispatched coding task"
        );

        // Spawn background execution.
        let svc = self.clone();
        let repo_url = repo_url.to_owned();
        let prompt = prompt.to_owned();
        tokio::spawn(async move {
            if let Err(e) = svc
                .run_task(task_id, &repo_url, &branch, &tmux_session, &prompt, agent_type)
                .await
            {
                warn!(id = %task_id, error = %e, "coding task execution failed");
                let _ = svc.repo.update_error(task_id, &e.to_string()).await;
                let _ = svc
                    .repo
                    .update_status(task_id, CodingTaskStatus::Failed)
                    .await;
                let _ = svc.repo.set_completed(task_id).await;
                svc.send_notification(
                    task_id, &prompt, &branch, agent_type, false, Some(&e.to_string()),
                )
                .await;
            }
        });

        Ok(task)
    }

    async fn run_task(
        &self,
        id: Uuid,
        repo_url: &str,
        branch: &str,
        tmux_session: &str,
        prompt: &str,
        agent_type: AgentType,
    ) -> Result<(), CodingTaskError> {
        // 1. Prepare workspace
        self.repo
            .update_status(id, CodingTaskStatus::Cloning)
            .await?;
        let repo_path = self
            .workspace_manager
            .ensure_repo(repo_url)
            .await
            .map_err(|e| WorkspaceSnafu { message: e.to_string() }.build())?;
        let worktree_path = self
            .workspace_manager
            .create_worktree(&repo_path, branch)
            .await
            .map_err(|e| WorkspaceSnafu { message: e.to_string() }.build())?;

        let wt_str = worktree_path.to_string_lossy().to_string();
        self.repo
            .update_workspace(id, &wt_str, tmux_session)
            .await?;

        // 2. Build agent command
        let agent_cmd = match agent_type {
            AgentType::Claude => {
                format!(
                    "cd '{}' && claude -p '{}' --output-format json",
                    wt_str,
                    prompt.replace('\'', "'\\''")
                )
            }
            AgentType::Codex => {
                format!(
                    "cd '{}' && codex exec --full-auto '{}'",
                    wt_str,
                    prompt.replace('\'', "'\\''")
                )
            }
        };

        // 3. Start tmux session
        self.repo
            .update_status(id, CodingTaskStatus::Running)
            .await?;
        self.repo.set_started(id).await?;

        let tmux_out = Command::new("tmux")
            .args(["new-session", "-d", "-s", tmux_session, &agent_cmd])
            .output()
            .await
            .map_err(|e| ExecutionSnafu { message: e.to_string() }.build())?;

        if !tmux_out.status.success() {
            let stderr = String::from_utf8_lossy(&tmux_out.stderr);
            return Err(ExecutionSnafu {
                message: format!("tmux new-session failed: {stderr}"),
            }
            .build());
        }

        // 4. Poll tmux session until it ends
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            let has = Command::new("tmux")
                .args(["has-session", "-t", tmux_session])
                .output()
                .await;
            match has {
                Ok(o) if o.status.success() => continue, // still running
                _ => break,                               // session ended
            }
        }

        // 5. Capture output (tmux pane history)
        let capture = Command::new("tmux")
            .args([
                "capture-pane",
                "-t",
                tmux_session,
                "-p",
                "-S",
                "-1000",
            ])
            .output()
            .await;
        let output = match capture {
            Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
            Err(_) => String::from("(could not capture output)"),
        };
        let output = tail_output(&output, 8192);
        self.repo.update_output(id, &output, Some(0)).await?;

        // 6. Create PR
        let pr_result = Command::new("gh")
            .args([
                "pr",
                "create",
                "--title",
                &format!("rara: {}", truncate(prompt, 60)),
                "--body",
                &format!("Automated PR from rara coding task `{}`\n\nPrompt:\n> {}", id, prompt),
                "--head",
                branch,
            ])
            .current_dir(&worktree_path)
            .output()
            .await;

        match pr_result {
            Ok(o) if o.status.success() => {
                let pr_url = String::from_utf8_lossy(&o.stdout).trim().to_owned();
                // Extract PR number from URL (last path segment)
                let pr_number = pr_url
                    .rsplit('/')
                    .next()
                    .and_then(|s| s.parse::<i32>().ok())
                    .unwrap_or(0);
                self.repo.update_pr(id, &pr_url, pr_number).await?;
            }
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr);
                warn!(id = %id, "gh pr create failed: {stderr}");
                // Not a fatal error — task itself completed.
            }
            Err(e) => {
                warn!(id = %id, "gh pr create error: {e}");
            }
        }

        // 7. Mark completed
        self.repo
            .update_status(id, CodingTaskStatus::Completed)
            .await?;
        self.repo.set_completed(id).await?;

        // 8. Send notification
        self.send_notification(id, prompt, branch, agent_type, true, None)
            .await;

        Ok(())
    }

    /// Get a single task by ID.
    pub async fn get(&self, id: Uuid) -> Result<CodingTask, CodingTaskError> {
        self.repo.get(id).await
    }

    /// List all tasks.
    pub async fn list(&self) -> Result<Vec<CodingTask>, CodingTaskError> {
        self.repo.list().await
    }

    /// Merge the PR for a completed task.
    pub async fn merge(&self, id: Uuid) -> Result<(), CodingTaskError> {
        let task = self.repo.get(id).await?;
        let pr_url = task.pr_url.as_deref().ok_or_else(|| {
            ExecutionSnafu {
                message: "no PR associated with this task".to_owned(),
            }
            .build()
        })?;

        let out = Command::new("gh")
            .args(["pr", "merge", pr_url, "--merge", "--delete-branch"])
            .output()
            .await
            .map_err(|e| ExecutionSnafu { message: e.to_string() }.build())?;

        if out.status.success() {
            self.repo
                .update_status(id, CodingTaskStatus::Merged)
                .await?;
        } else {
            let stderr = String::from_utf8_lossy(&out.stderr);
            self.repo
                .update_error(id, &format!("merge failed: {stderr}"))
                .await?;
            self.repo
                .update_status(id, CodingTaskStatus::MergeFailed)
                .await?;
        }
        Ok(())
    }

    /// Cancel a running task by killing its tmux session.
    pub async fn cancel(&self, id: Uuid) -> Result<(), CodingTaskError> {
        let task = self.repo.get(id).await?;
        if !task.tmux_session.is_empty() {
            let _ = Command::new("tmux")
                .args(["kill-session", "-t", &task.tmux_session])
                .output()
                .await;
        }
        self.repo
            .update_status(id, CodingTaskStatus::Failed)
            .await?;
        self.repo
            .update_error(id, "cancelled by user")
            .await?;
        self.repo.set_completed(id).await?;
        Ok(())
    }

    async fn send_notification(
        &self,
        id: Uuid,
        prompt: &str,
        branch: &str,
        agent_type: AgentType,
        success: bool,
        error_msg: Option<&str>,
    ) {
        let emoji = if success { "\u{2705}" } else { "\u{274c}" };
        let status_label = if success { "completed" } else { "failed" };
        let short_id = &id.to_string()[..8];
        let prompt_short = truncate(prompt, 100);
        let mut msg = format!(
            "{emoji} Coding task {short_id} {status_label}\nBranch: {branch}\nAgent: \
             {agent_type}\nPrompt: \"{prompt_short}\""
        );
        if let Some(err) = error_msg {
            let tail = truncate(err, 500);
            msg.push_str(&format!("\n\nError:\n{tail}"));
        }

        let settings = self.settings_svc.current();
        let chat_id = settings.telegram.chat_id;

        let request = SendTelegramNotificationRequest {
            chat_id,
            subject: None,
            body: msg,
            priority: NotificationPriority::Normal,
            max_retries: 3,
            reference_type: None,
            reference_id: None,
            metadata: None,
            photo_path: None,
        };
        if let Err(e) = self.notify.send_telegram(request).await {
            warn!(id = %id, "failed to send coding task notification: {e}");
        }
    }
}

/// Keep only the last `max_bytes` of output.
fn tail_output(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_owned();
    }
    let mut start = s.len() - max_bytes;
    while !s.is_char_boundary(start) && start < s.len() {
        start += 1;
    }
    s[start..].to_owned()
}

/// Truncate a string to at most `max` characters.
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_owned()
    } else {
        let mut end = max;
        while !s.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

/// Convenience constructor for wiring in the app composition root.
pub fn wire(
    pool: sqlx::PgPool,
    workspace_manager: WorkspaceManager,
    notify: NotifyClient,
    settings_svc: SettingsSvc,
    default_repo_url: String,
) -> CodingTaskService {
    let repo = Arc::new(crate::pg_repository::PgCodingTaskRepository::new(pool));
    CodingTaskService::new(repo, workspace_manager, notify, settings_svc, default_repo_url)
}
