use std::path::Path;
use std::time::Instant;

use snafu::ResultExt;
use tokio::process::{Child, Command};

use crate::config::AgentConfig;
use crate::error::{IoSnafu, Result};
use crate::tracker::TrackedIssue;

#[derive(Debug, Clone)]
pub struct AgentTask {
    pub issue: TrackedIssue,
    pub attempt: Option<u32>,
    pub workflow_content: Option<String>,
}

#[derive(Debug)]
pub struct AgentHandle {
    pub child: Child,
    pub started_at: Instant,
}

#[derive(Debug, Clone)]
pub struct RalphAgent {
    config: AgentConfig,
}

impl RalphAgent {
    #[must_use]
    pub fn new(config: AgentConfig) -> Self {
        Self { config }
    }

    pub fn build_prompt(&self, task: &AgentTask) -> String {
        if let Some(content) = &task.workflow_content {
            let trimmed = content.trim();
            if !trimmed.is_empty() {
                // Repository-local workflow content defines the task body; we append
                // the required delivery contract so symphony can rely on a stable
                // operational workflow across repos.
                return format!("{trimmed}\n\n{}", self.required_outcome_section(task));
            }
        }

        let body_section = task.issue.body.as_deref().map_or(String::new(), |body| {
            format!("\n## Description\n\n{body}\n")
        });

        let retry_section = task.attempt.map_or(String::new(), |attempt| {
            format!(
                "\nThis is retry attempt {attempt}. The previous attempt failed. Review the prior attempt and take a different approach.\n"
            )
        });

        format!(
            "\
Issue #{number}: {title}
{body_section}{retry_section}
## Instructions

- Work in the current working directory.
- Make the requested code changes for this issue.
- Run relevant verification before finishing.
-{required_outcome}",
            number = task.issue.number,
            title = task.issue.title,
            required_outcome = self.required_outcome_section(task),
        )
    }

    fn required_outcome_section(&self, task: &AgentTask) -> String {
        format!(
            "\
## Required Delivery

- commit your changes.
- push the branch to the remote repository.
- create a GitHub pull request for this branch before finishing.
- comment on the Linear issue with the GitHub pull request link before finishing.
- include the issue identifier `{}` in the commit message and pull request title or body.
",
            task.issue.identifier
        )
    }

    /// Write `PROMPT.md` into the issue worktree and spawn a non-interactive
    /// `ralph run` subprocess whose output is consumed by symphony.
    pub async fn start<P: AsRef<Path>>(&self, task: &AgentTask, workspace: P) -> Result<AgentHandle> {
        let prompt_path = workspace.as_ref().join("PROMPT.md");
        tokio::fs::write(&prompt_path, self.build_prompt(task))
            .await
            .context(IoSnafu)?;

        let mut cmd = Command::new(&self.config.command);
        for arg in self.config.command_args() {
            cmd.arg(arg);
        }

        cmd.current_dir(workspace.as_ref());
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let child = cmd.spawn().context(IoSnafu)?;

        Ok(AgentHandle {
            child,
            started_at: Instant::now(),
        })
    }
}
