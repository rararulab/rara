use std::time::Instant;

use async_trait::async_trait;
use tokio::process::{Child, Command};

use crate::config::AgentConfig;
use crate::error::{IoSnafu, Result};
use crate::event::{TrackedIssue, WorkspaceInfo};
use crate::workflow::{self, PromptContext};

/// A task to be executed by a coding agent.
#[derive(Debug, Clone)]
pub struct AgentTask {
    /// The issue this task is for.
    pub issue: TrackedIssue,
    /// Retry attempt number (`None` for first attempt, `Some(n)` for retries).
    pub attempt: Option<u32>,
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
    ///
    /// If the task has workflow content, it is parsed as a workflow file and
    /// rendered as a template with issue + attempt context. If the workflow
    /// template body is empty or absent, a default fallback prompt is used.
    pub fn build_prompt(&self, task: &AgentTask) -> Result<String> {
        let ctx = PromptContext {
            issue: &task.issue,
            attempt: task.attempt,
        };

        // Try to use the workflow template if provided.
        if let Some(content) = &task.workflow_content {
            let wf = workflow::parse_workflow(content)?;
            if !wf.prompt_template.is_empty() {
                return workflow::render_prompt(&wf.prompt_template, &ctx);
            }
        }

        // Fallback: build a default prompt.
        Ok(self.default_prompt(task))
    }

    /// Build the default hardcoded prompt when no workflow template is available.
    fn default_prompt(&self, task: &AgentTask) -> String {
        let body_section = task.issue.body.as_deref().map_or(String::new(), |body| {
            format!("\n## Description\n\n{body}\n")
        });

        let retry_section = task.attempt.map_or(String::new(), |attempt| {
            format!(
                "\nThis is retry attempt {attempt}. \
                 The previous attempt failed. Please review what went wrong and try a different approach.\n"
            )
        });

        format!(
            "\
Issue #{number}: {title}
{body_section}{retry_section}
## Instructions

- Work in the current working directory (the worktree).
- Use conventional commits (feat, fix, refactor, etc.).
- Include issue reference (#{number}) in commit messages.
- When finished, create a PR with your changes.
",
            number = task.issue.number,
            title = task.issue.title,
        )
    }
}

#[async_trait]
impl CodingAgent for ClaudeCodeAgent {
    async fn start(&self, task: &AgentTask, workspace: &WorkspaceInfo) -> Result<AgentHandle> {
        use snafu::ResultExt;

        let prompt = self.build_prompt(task)?;

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
            identifier: "42".to_owned(),
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
    fn build_prompt_with_workflow_template() {
        let agent = ClaudeCodeAgent::new(
            AgentConfig::builder()
                .command("claude".to_owned())
                .args(vec![])
                .allowed_tools(vec![])
                .build(),
        );

        let task = AgentTask {
            issue: sample_issue(),
            attempt: None,
            workflow_content: Some(
                "You are working on issue #{{issue.number}}: {{issue.title}}\n\n{{issue.body}}"
                    .to_owned(),
            ),
        };

        let prompt = agent.build_prompt(&task).unwrap();

        assert!(prompt.contains("issue #42: Add widget support"));
        assert!(prompt.contains("We need widgets for the dashboard."));
        // Should NOT contain the default instructions since workflow template is used.
        assert!(!prompt.contains("conventional commits"));
    }

    #[test]
    fn build_prompt_fallback_without_workflow() {
        let agent = ClaudeCodeAgent::new(
            AgentConfig::builder()
                .command("claude".to_owned())
                .args(vec![])
                .allowed_tools(vec![])
                .build(),
        );

        let task = AgentTask {
            issue: sample_issue(),
            attempt: None,
            workflow_content: None,
        };

        let prompt = agent.build_prompt(&task).unwrap();

        assert!(prompt.contains("Issue #42: Add widget support"));
        assert!(prompt.contains("We need widgets for the dashboard."));
        assert!(prompt.contains("conventional commits"));
        assert!(prompt.contains("#42"));
    }

    #[test]
    fn build_prompt_fallback_with_empty_workflow_body() {
        let agent = ClaudeCodeAgent::new(
            AgentConfig::builder()
                .command("claude".to_owned())
                .args(vec![])
                .allowed_tools(vec![])
                .build(),
        );

        let task = AgentTask {
            issue: sample_issue(),
            attempt: None,
            workflow_content: Some("---\nkey: value\n---\n".to_owned()),
        };

        let prompt = agent.build_prompt(&task).unwrap();

        // Empty workflow body should fall back to default prompt.
        assert!(prompt.contains("Issue #42"));
        assert!(prompt.contains("conventional commits"));
    }

    #[test]
    fn build_prompt_with_retry_attempt() {
        let agent = ClaudeCodeAgent::new(
            AgentConfig::builder()
                .command("claude".to_owned())
                .args(vec![])
                .allowed_tools(vec![])
                .build(),
        );

        let task = AgentTask {
            issue: sample_issue(),
            attempt: Some(2),
            workflow_content: None,
        };

        let prompt = agent.build_prompt(&task).unwrap();

        assert!(prompt.contains("retry attempt 2"));
    }

    #[test]
    fn build_prompt_without_body() {
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
            attempt: None,
            workflow_content: None,
        };

        let prompt = agent.build_prompt(&task).unwrap();

        assert!(prompt.contains("Issue #42"));
        assert!(!prompt.contains("## Description"));
    }
}
