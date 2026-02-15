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

//! Layer 2 service tools for dispatching CLI coding agents (Claude Code / Codex).

use std::{
    fmt,
    path::PathBuf,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
    time::Instant,
};

use async_trait::async_trait;
use rara_agents::tool_registry::AgentTool;
use rara_domain_shared::{
    notify::{
        client::NotifyClient,
        types::{NotificationPriority, SendTelegramNotificationRequest},
    },
    settings::SettingsSvc,
};
use serde_json::json;
use tokio::sync::RwLock;
use tracing::{info, warn};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Which CLI agent to invoke.
#[derive(Debug, Clone, Copy)]
pub enum AgentKind {
    Claude,
    Codex,
}

impl fmt::Display for AgentKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Claude => write!(f, "claude"),
            Self::Codex => write!(f, "codex"),
        }
    }
}

/// Current status of a dispatched agent task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Running,
    Completed,
    Failed,
}

impl fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

/// A single dispatched coding task.
#[derive(Debug, Clone)]
pub struct AgentTask {
    pub id:            u32,
    pub prompt:        String,
    pub agent:         AgentKind,
    pub branch:        String,
    pub worktree_path: PathBuf,
    pub status:        TaskStatus,
    /// Tail of stdout+stderr (max ~4 KB).
    pub output:        String,
    pub exit_code:     Option<i32>,
    pub started_at:    Instant,
    pub finished_at:   Option<Instant>,
}

// ---------------------------------------------------------------------------
// AgentTaskStore
// ---------------------------------------------------------------------------

/// Thread-safe in-memory store for dispatched agent tasks.
#[derive(Clone)]
pub struct AgentTaskStore {
    tasks:   Arc<RwLock<Vec<AgentTask>>>,
    next_id: Arc<AtomicU32>,
}

impl AgentTaskStore {
    #[must_use]
    pub fn new() -> Self {
        Self {
            tasks:   Arc::new(RwLock::new(Vec::new())),
            next_id: Arc::new(AtomicU32::new(1)),
        }
    }

    /// Insert a new task and return its ID.
    pub async fn add(&self, task: AgentTask) -> u32 {
        let id = task.id;
        self.tasks.write().await.push(task);
        id
    }

    /// Allocate the next unique task ID.
    pub fn next_id(&self) -> u32 { self.next_id.fetch_add(1, Ordering::Relaxed) }

    /// Get a task by ID.
    pub async fn get(&self, id: u32) -> Option<AgentTask> {
        self.tasks.read().await.iter().find(|t| t.id == id).cloned()
    }

    /// Get the most recently added task.
    pub async fn get_latest(&self) -> Option<AgentTask> {
        self.tasks.read().await.last().cloned()
    }

    /// Return summaries of all tasks.
    pub async fn list(&self) -> Vec<serde_json::Value> {
        self.tasks
            .read()
            .await
            .iter()
            .map(|t| {
                json!({
                    "id": t.id,
                    "agent": t.agent.to_string(),
                    "branch": t.branch,
                    "status": t.status.to_string(),
                    "prompt": truncate(&t.prompt, 100),
                    "elapsed_secs": t.finished_at
                        .unwrap_or_else(Instant::now)
                        .duration_since(t.started_at)
                        .as_secs(),
                })
            })
            .collect()
    }

    /// Update a task's status, output, and exit code.
    pub async fn update_status(
        &self,
        id: u32,
        status: TaskStatus,
        output: String,
        exit_code: Option<i32>,
    ) {
        let mut tasks = self.tasks.write().await;
        if let Some(task) = tasks.iter_mut().find(|t| t.id == id) {
            task.status = status;
            task.output = output;
            task.exit_code = exit_code;
            task.finished_at = Some(Instant::now());
        }
    }
}

/// Truncate a string to at most `max` characters, appending "..." if
/// truncated.
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_owned()
    } else {
        let mut end = max;
        // Avoid splitting a multi-byte character.
        while !s.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

/// Keep only the last ~4 KB of output.
fn tail_output(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_owned();
    }
    let start = s.len() - max_bytes;
    // Avoid splitting a multi-byte character.
    let mut start = start;
    while !s.is_char_boundary(start) && start < s.len() {
        start += 1;
    }
    s[start..].to_owned()
}

// ---------------------------------------------------------------------------
// CodexRunTool
// ---------------------------------------------------------------------------

/// Dispatch a coding task to a CLI agent (Claude Code or Codex).
pub struct CodexRunTool {
    store:        AgentTaskStore,
    notify:       NotifyClient,
    settings_svc: SettingsSvc,
    project_root: PathBuf,
}

impl CodexRunTool {
    pub fn new(
        store: AgentTaskStore,
        notify: NotifyClient,
        settings_svc: SettingsSvc,
        project_root: PathBuf,
    ) -> Self {
        Self {
            store,
            notify,
            settings_svc,
            project_root,
        }
    }
}

#[async_trait]
impl AgentTool for CodexRunTool {
    fn name(&self) -> &str { "codex_run" }

    fn description(&self) -> &str {
        "Dispatch a coding task to a CLI agent (Claude Code or Codex). Creates a git worktree for \
         isolation and runs the agent in the background. Returns immediately with a task ID. Sends \
         a Telegram notification when the task completes."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "The coding task prompt to send to the agent"
                },
                "agent": {
                    "type": "string",
                    "enum": ["claude", "codex"],
                    "description": "Which CLI agent to use (default: claude)"
                },
                "branch": {
                    "type": "string",
                    "description": "Git branch name for the worktree (auto-generated if omitted)"
                }
            },
            "required": ["prompt"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
    ) -> rara_agents::err::Result<serde_json::Value> {
        let prompt = params
            .get("prompt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| rara_agents::err::Error::Other {
                message: "missing required parameter: prompt".into(),
            })?
            .to_owned();

        let agent_kind = match params.get("agent").and_then(|v| v.as_str()) {
            Some("codex") => AgentKind::Codex,
            _ => AgentKind::Claude,
        };

        let id = self.store.next_id();

        let branch = params
            .get("branch")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| format!("rara-task-{id}"));

        let worktree_path = self.project_root.join(".worktrees").join(&branch);

        // Create the git worktree.
        let worktree_output = tokio::process::Command::new("git")
            .args(["worktree", "add"])
            .arg(&worktree_path)
            .args(["-b", &branch, "main"])
            .current_dir(&self.project_root)
            .output()
            .await
            .map_err(|e| rara_agents::err::Error::Other {
                message: format!("failed to run git worktree add: {e}").into(),
            })?;

        if !worktree_output.status.success() {
            let stderr = String::from_utf8_lossy(&worktree_output.stderr);
            return Err(rara_agents::err::Error::Other {
                message: format!("git worktree add failed: {stderr}").into(),
            });
        }

        let task = AgentTask {
            id,
            prompt: prompt.clone(),
            agent: agent_kind,
            branch: branch.clone(),
            worktree_path: worktree_path.clone(),
            status: TaskStatus::Running,
            output: String::new(),
            exit_code: None,
            started_at: Instant::now(),
            finished_at: None,
        };

        self.store.add(task).await;
        info!(id, branch = %branch, agent = %agent_kind, "dispatched codex agent task");

        // Spawn background task.
        let store = self.store.clone();
        let notify = self.notify.clone();
        let settings_svc = self.settings_svc.clone();
        let prompt_clone = prompt.clone();
        let branch_clone = branch.clone();
        let wt = worktree_path.clone();

        tokio::spawn(async move {
            let mut cmd = match agent_kind {
                AgentKind::Claude => {
                    let mut c = tokio::process::Command::new("claude");
                    c.args(["-p", &prompt_clone, "--output-format", "json"]);
                    c
                }
                AgentKind::Codex => {
                    let mut c = tokio::process::Command::new("codex");
                    c.args(["-q", &prompt_clone, "--auto-edit"]);
                    c
                }
            };

            cmd.current_dir(&wt);
            cmd.stdout(std::process::Stdio::piped());
            cmd.stderr(std::process::Stdio::piped());

            let result = cmd.output().await;

            let (status, output, exit_code) = match result {
                Ok(out) => {
                    let combined = format!(
                        "{}\n{}",
                        String::from_utf8_lossy(&out.stdout),
                        String::from_utf8_lossy(&out.stderr)
                    );
                    let output = tail_output(&combined, 4096);
                    let code = out.status.code();
                    if out.status.success() {
                        (TaskStatus::Completed, output, code)
                    } else {
                        (TaskStatus::Failed, output, code)
                    }
                }
                Err(e) => (
                    TaskStatus::Failed,
                    format!("failed to spawn process: {e}"),
                    None,
                ),
            };

            let elapsed = store
                .get(id)
                .await
                .map(|t| {
                    let dur = Instant::now().duration_since(t.started_at);
                    humanize_duration(dur)
                })
                .unwrap_or_else(|| "?".to_owned());

            store
                .update_status(id, status, output.clone(), exit_code)
                .await;

            // Send TG notification.
            let emoji = if status == TaskStatus::Completed {
                "\u{2705}" // checkmark
            } else {
                "\u{274c}" // cross mark
            };
            let status_label = if status == TaskStatus::Completed {
                "completed"
            } else {
                "failed"
            };
            let prompt_short = truncate(&prompt_clone, 100);
            let mut msg = format!(
                "{emoji} Coding task #{id} {status_label} ({elapsed})\nBranch: \
                 {branch_clone}\nAgent: {agent_kind}\nPrompt: \"{prompt_short}\""
            );
            if status == TaskStatus::Failed {
                let tail = truncate(&output, 500);
                msg.push_str(&format!("\n\nError output:\n{tail}"));
            }

            let settings = settings_svc.current();
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
            if let Err(e) = notify.send_telegram(request).await {
                warn!(id, "failed to send codex task notification: {e}");
            }
        });

        Ok(json!({
            "task_id": id,
            "branch": branch,
            "status": "running",
            "worktree": worktree_path.to_string_lossy(),
        }))
    }
}

// ---------------------------------------------------------------------------
// CodexStatusTool
// ---------------------------------------------------------------------------

/// Check status and output of a dispatched coding task.
pub struct CodexStatusTool {
    store: AgentTaskStore,
}

impl CodexStatusTool {
    pub fn new(store: AgentTaskStore) -> Self { Self { store } }
}

#[async_trait]
impl AgentTool for CodexStatusTool {
    fn name(&self) -> &str { "codex_status" }

    fn description(&self) -> &str {
        "Check status and output of a dispatched coding task. If no task_id is provided, returns \
         the latest task."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "number",
                    "description": "Task ID to check (returns latest if omitted)"
                }
            }
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
    ) -> rara_agents::err::Result<serde_json::Value> {
        let task = if let Some(id) = params.get("task_id").and_then(|v| v.as_u64()) {
            self.store.get(id as u32).await
        } else {
            self.store.get_latest().await
        };

        match task {
            Some(t) => {
                let elapsed = t
                    .finished_at
                    .unwrap_or_else(Instant::now)
                    .duration_since(t.started_at);
                Ok(json!({
                    "id": t.id,
                    "agent": t.agent.to_string(),
                    "branch": t.branch,
                    "worktree": t.worktree_path.to_string_lossy(),
                    "status": t.status.to_string(),
                    "prompt": t.prompt,
                    "output": t.output,
                    "exit_code": t.exit_code,
                    "elapsed_secs": elapsed.as_secs(),
                }))
            }
            None => Ok(json!({
                "error": "no tasks found"
            })),
        }
    }
}

// ---------------------------------------------------------------------------
// CodexListTool
// ---------------------------------------------------------------------------

/// List all dispatched coding tasks and their status.
pub struct CodexListTool {
    store: AgentTaskStore,
}

impl CodexListTool {
    pub fn new(store: AgentTaskStore) -> Self { Self { store } }
}

#[async_trait]
impl AgentTool for CodexListTool {
    fn name(&self) -> &str { "codex_list" }

    fn description(&self) -> &str { "List all dispatched coding tasks and their status." }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
    ) -> rara_agents::err::Result<serde_json::Value> {
        let items = self.store.list().await;
        Ok(json!({
            "count": items.len(),
            "tasks": items,
        }))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Format a duration as a human-readable string.
fn humanize_duration(dur: std::time::Duration) -> String {
    let secs = dur.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}
