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

use std::{path::Path, process::Stdio, time::Instant};

use serde_yaml::{Mapping, Value};
use snafu::ResultExt;
use tokio::process::{Child, Command};
use tracing::{info, warn};

use crate::{
    config::AgentConfig,
    error::{ConfigYamlSnafu, IoSnafu, Result},
    tracker::TrackedIssue,
};

#[derive(Debug, Clone)]
pub struct AgentTask {
    pub issue:            TrackedIssue,
    pub attempt:          Option<u32>,
    pub workflow_content: Option<String>,
}

#[derive(Debug)]
pub struct AgentHandle {
    pub child:      Child,
    pub started_at: Instant,
}

#[derive(Debug, Clone)]
pub struct RalphAgent {
    config: AgentConfig,
}

// `ralph init -c <core>` uses the extra config while generating defaults, but
// does not write those overrides back into the resulting `ralph.yml`. We merge
// the repo-maintained core config into the generated file so later `ralph run`
// can rely on the worktree-local config alone.
pub fn merge_core_config(generated: &str, core: &str) -> Result<String> {
    let generated_value: Value = serde_yaml::from_str(generated).context(ConfigYamlSnafu {
        message: String::from("failed to parse generated ralph.yml"),
    })?;
    let core_value: Value = serde_yaml::from_str(core).context(ConfigYamlSnafu {
        message: String::from("failed to parse Ralph core config"),
    })?;

    let mut generated_map = match generated_value {
        Value::Mapping(map) => map,
        _ => Mapping::new(),
    };
    let core_map = match core_value {
        Value::Mapping(map) => map,
        _ => Mapping::new(),
    };

    for (key, value) in core_map {
        generated_map.insert(key, value);
    }

    serde_yaml::to_string(&Value::Mapping(generated_map)).context(ConfigYamlSnafu {
        message: String::from("failed to serialize merged Ralph config"),
    })
}

impl RalphAgent {
    #[must_use]
    pub fn new(config: AgentConfig) -> Self { Self { config } }

    async fn doctor_workspace<P: AsRef<Path>>(&self, workspace: P) -> Result<()> {
        let mut cmd = Command::new(&self.config.command);
        for arg in self.config.doctor_args() {
            cmd.arg(arg);
        }

        let output = cmd
            .current_dir(workspace.as_ref())
            .stdin(Stdio::null())
            .output()
            .await
            .context(IoSnafu)?;

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();

        if !stdout.is_empty() {
            info!(output = %stdout, "ralph doctor stdout");
        }
        if !stderr.is_empty() {
            if output.status.success() {
                info!(output = %stderr, "ralph doctor stderr");
            } else {
                warn!(output = %stderr, "ralph doctor stderr");
            }
        }

        if output.status.success() {
            return Ok(());
        }

        let details = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("ralph doctor exited with {}", output.status)
        };

        Err(crate::error::SymphonyError::Workspace {
            message:  format!("failed to validate Ralph workspace config: {details}"),
            location: snafu::Location::new(file!(), line!(), column!()),
        })
    }

    async fn init_workspace_config<P: AsRef<Path>>(&self, workspace: P) -> Result<()> {
        let mut cmd = Command::new(&self.config.command);
        for arg in self.config.init_args() {
            cmd.arg(arg);
        }

        let output = cmd
            .current_dir(workspace.as_ref())
            .stdin(std::process::Stdio::null())
            .output()
            .await
            .context(IoSnafu)?;

        if output.status.success() {
            // Materialize the repo root core config into the generated
            // worktree-local `ralph.yml` because the later `ralph run` step
            // intentionally executes without extra `-c` overlays.
            let generated_path = workspace.as_ref().join("ralph.yml");
            let core_path = workspace.as_ref().join(&self.config.core_config_file);
            let generated = tokio::fs::read_to_string(&generated_path)
                .await
                .context(IoSnafu)?;
            let core = tokio::fs::read_to_string(&core_path)
                .await
                .context(IoSnafu)?;
            let merged = merge_core_config(&generated, &core)?;
            tokio::fs::write(&generated_path, merged)
                .await
                .context(IoSnafu)?;
            self.doctor_workspace(workspace.as_ref()).await?;
            return Ok(());
        }

        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        let details = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("ralph init exited with {}", output.status)
        };

        Err(crate::error::SymphonyError::Workspace {
            message:  format!("failed to initialize Ralph workspace config: {details}"),
            location: snafu::Location::new(file!(), line!(), column!()),
        })
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
                "\nThis is retry attempt {attempt}. The previous attempt failed. Review the prior \
                 attempt and take a different approach.\n"
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

- use the system-installed linear CLI (`linear`) to comment on the Linear issue before or during \
             implementation.
- in that Linear comment, summarize your reasoning and implementation plan for the issue.
- commit your changes.
- push the branch to the remote repository.
- create a GitHub pull request for this branch before finishing.
- use the same linear CLI (`linear`) to comment on the Linear issue again with the GitHub pull \
             request link and a short implementation summary before finishing.
- include the issue identifier `{}` in the commit message and pull request title or body.
",
            task.issue.identifier
        )
    }

    /// Write `PROMPT.md` into the issue worktree and spawn a non-interactive
    /// `ralph run` subprocess whose output is consumed by symphony.
    pub async fn start<P: AsRef<Path>>(
        &self,
        task: &AgentTask,
        workspace: P,
    ) -> Result<AgentHandle> {
        self.init_workspace_config(workspace.as_ref()).await?;

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
