use std::time::Instant;

use async_trait::async_trait;
use tokio::process::{Child, Command};

use crate::config::AgentConfig;
use crate::error::{IoSnafu, Result};
use crate::event::{TrackedIssue, WorkspaceInfo};

/// A task to be executed by a coding agent.
#[derive(Debug, Clone)]
pub struct AgentTask {
    /// The issue this task is for.
    pub issue: TrackedIssue,
    /// The prompt to send to the agent.
    pub prompt: String,
    /// Optional workflow file content to include in the prompt.
    pub workflow_content: Option<String>,
}

/// Handle to a running coding agent process.
#[derive(Debug)]
pub struct AgentHandle {
    /// The child process.
    pub child: Child,
    /// When the agent was started.
    pub started_at: Instant,
}

/// Trait for spawning coding agents that work on issues.
#[async_trait]
pub trait CodingAgent: Send + Sync {
    /// Start an agent to work on the given task in the given workspace.
    async fn start(&self, task: &AgentTask, workspace: &WorkspaceInfo) -> Result<AgentHandle>;
}

/// A coding agent backed by Claude Code CLI.
#[derive(Debug, Clone)]
pub struct ClaudeCodeAgent {
    config: AgentConfig,
}

impl ClaudeCodeAgent {
    /// Create a new `ClaudeCodeAgent` with the given configuration.
    #[must_use]
    pub fn new(config: AgentConfig) -> Self {
        Self { config }
    }

    /// Build the full prompt string for an agent task.
    #[must_use]
    pub fn build_prompt(&self, task: &AgentTask) -> String {
        let mut prompt = format!(
            "Issue #{}: {}\n",
            task.issue.number, task.issue.title
        );

        if let Some(body) = &task.issue.body {
            prompt.push_str("\n## Description\n\n");
            prompt.push_str(body);
            prompt.push('\n');
        }

        if let Some(workflow) = &task.workflow_content {
            prompt.push_str("\n## Workflow\n\n");
            prompt.push_str(workflow);
            prompt.push('\n');
        }

        prompt.push_str("\n## Instructions\n\n");
        prompt.push_str("- Work in the current working directory (the worktree).\n");
        prompt.push_str("- Use conventional commits (feat, fix, refactor, etc.).\n");
        prompt.push_str(&format!(
            "- Include issue reference (#{}) in commit messages.\n",
            task.issue.number
        ));
        prompt.push_str("- When finished, create a PR with your changes.\n");

        prompt
    }
}

#[async_trait]
impl CodingAgent for ClaudeCodeAgent {
    async fn start(&self, task: &AgentTask, workspace: &WorkspaceInfo) -> Result<AgentHandle> {
        use snafu::ResultExt;

        let prompt = self.build_prompt(task);

        let mut cmd = Command::new(&self.config.command);

        for arg in &self.config.args {
            cmd.arg(arg);
        }

        // Add allowed tools if configured.
        for tool in &self.config.allowed_tools {
            cmd.arg("--allowedTools").arg(tool);
        }

        cmd.arg("--print").arg(&prompt);

        cmd.current_dir(&workspace.path);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let child = cmd.spawn().context(IoSnafu)?;

        Ok(AgentHandle {
            child,
            started_at: Instant::now(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn sample_issue() -> TrackedIssue {
        TrackedIssue {
            id: "owner/repo#42".to_owned(),
            repo: "owner/repo".to_owned(),
            number: 42,
            title: "Add widget support".to_owned(),
            body: Some("We need widgets for the dashboard.".to_owned()),
            labels: vec!["enhancement".to_owned()],
            priority: 1,
            state: crate::event::IssueState::Active,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn build_prompt_includes_issue_info() {
        let agent = ClaudeCodeAgent::new(
            AgentConfig::builder()
                .command("claude".to_owned())
                .args(vec![])
                .allowed_tools(vec![])
                .build(),
        );

        let task = AgentTask {
            issue: sample_issue(),
            prompt: String::new(),
            workflow_content: Some("Step 1: do stuff".to_owned()),
        };

        let prompt = agent.build_prompt(&task);

        assert!(prompt.contains("Issue #42: Add widget support"));
        assert!(prompt.contains("We need widgets for the dashboard."));
        assert!(prompt.contains("Step 1: do stuff"));
        assert!(prompt.contains("conventional commits"));
        assert!(prompt.contains("#42"));
    }

    #[test]
    fn build_prompt_without_optional_fields() {
        let agent = ClaudeCodeAgent::new(
            AgentConfig::builder()
                .command("claude".to_owned())
                .args(vec![])
                .allowed_tools(vec![])
                .build(),
        );

        let mut issue = sample_issue();
        issue.body = None;

        let task = AgentTask {
            issue,
            prompt: String::new(),
            workflow_content: None,
        };

        let prompt = agent.build_prompt(&task);

        assert!(prompt.contains("Issue #42"));
        assert!(!prompt.contains("## Description"));
        assert!(!prompt.contains("## Workflow"));
    }
}
