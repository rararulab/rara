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
use snafu::{IntoError, ResultExt};
use tokio::process::{Child, Command};
use tracing::{info, warn};

use crate::{
    config::AgentConfig,
    error::{ConfigYamlSnafu, Result, WorkspaceIoSnafu},
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
    /// The spawned ralph subprocess.
    pub child:      Child,
    /// Piped stdin handle for sending RPC messages to the agent.
    pub stdin:      Option<tokio::process::ChildStdin>,
    /// When the agent was spawned.
    pub started_at: Instant,
}

#[derive(Debug, Clone)]
pub struct RalphAgent {
    config: AgentConfig,
}

/// Merge repository-maintained core config into the `ralph init`-generated
/// config.
///
/// This is a **shallow merge**: top-level keys from `core` overwrite the
/// corresponding keys in `generated` entirely. Nested mappings (e.g. `agent:`)
/// are replaced, not deep-merged. This is intentional — the core config is
/// authoritative for any key it defines.
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

    fn format_command(&self, args: &[String]) -> String {
        let mut parts = Vec::with_capacity(args.len() + 1);
        parts.push(self.config.command.clone());
        parts.extend(args.iter().cloned());
        parts.join(" ")
    }

    async fn merge_core_config_if_present<P: AsRef<Path>>(&self, workspace: P) -> Result<()> {
        let generated_path = workspace.as_ref().join("ralph.yml");
        let core_path = workspace.as_ref().join(&self.config.core_config_file);
        let generated =
            tokio::fs::read_to_string(&generated_path)
                .await
                .context(WorkspaceIoSnafu {
                    message: format!(
                        "failed to read generated Ralph config {}",
                        generated_path.display()
                    ),
                })?;

        let core = match tokio::fs::read_to_string(&core_path).await {
            Ok(core) => core,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                // Dynamically discovered repos are not required to carry a
                // repo-local `ralph.core.yml`; in that case the generated
                // worktree-local `ralph.yml` is the best available config.
                info!(
                    path = %core_path.display(),
                    "Ralph core config missing in worktree; using generated ralph.yml as-is"
                );
                return Ok(());
            }
            Err(source) => {
                return Err(WorkspaceIoSnafu {
                    message: format!("failed to read Ralph core config {}", core_path.display()),
                }
                .into_error(source));
            }
        };

        let merged = merge_core_config(&generated, &core)?;
        tokio::fs::write(&generated_path, merged)
            .await
            .context(WorkspaceIoSnafu {
                message: format!(
                    "failed to write merged Ralph config {}",
                    generated_path.display()
                ),
            })?;
        Ok(())
    }

    async fn doctor_workspace<P: AsRef<Path>>(&self, workspace: P) -> Result<()> {
        let args = self.config.doctor_args();
        let command = self.format_command(&args);
        let mut cmd = Command::new(&self.config.command);
        for arg in &args {
            cmd.arg(arg);
        }

        let output = cmd
            .current_dir(workspace.as_ref())
            .stdin(Stdio::null())
            .output()
            .await
            .context(WorkspaceIoSnafu {
                message: format!(
                    "failed to run `{command}` in {}",
                    workspace.as_ref().display()
                ),
            })?;

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

        crate::error::WorkspaceSnafu {
            message: format!("failed to validate Ralph workspace config: {details}"),
        }
        .fail()
    }

    async fn init_workspace_config<P: AsRef<Path>>(&self, workspace: P) -> Result<()> {
        let args = self.config.init_args();
        let command = self.format_command(&args);
        let mut cmd = Command::new(&self.config.command);
        for arg in &args {
            cmd.arg(arg);
        }

        let output = cmd
            .current_dir(workspace.as_ref())
            .stdin(std::process::Stdio::null())
            .output()
            .await
            .context(WorkspaceIoSnafu {
                message: format!(
                    "failed to run `{command}` in {}",
                    workspace.as_ref().display()
                ),
            })?;

        if output.status.success() {
            // Materialize the repo root core config into the generated
            // worktree-local `ralph.yml` because the later `ralph run` step
            // intentionally executes without extra `-c` overlays.
            self.merge_core_config_if_present(workspace.as_ref())
                .await?;
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

        crate::error::WorkspaceSnafu {
            message: format!("failed to initialize Ralph workspace config: {details}"),
        }
        .fail()
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

    /// Spawn a `ralph run --rpc` review session for an issue's branch.
    pub async fn start_review<P: AsRef<Path>>(
        &self,
        issue: &TrackedIssue,
        workspace: P,
        review_config: &crate::config::ReviewConfig,
    ) -> Result<AgentHandle> {
        let branch = format!("issue-{}-{}", issue.number, slug(&issue.title));
        let prompt = format!(
            "Review the changes on branch `{branch}` for issue #{number} ({title}).\nCheck \
             correctness, test coverage, and code quality.\nIf a GitHub PR exists, use `gh pr \
             diff` to get the diff.",
            number = issue.number,
            title = issue.title,
        );

        let prompt_path = workspace.as_ref().join("PROMPT.md");
        tokio::fs::write(&prompt_path, &prompt)
            .await
            .context(WorkspaceIoSnafu {
                message: format!("failed to write review prompt {}", prompt_path.display()),
            })?;

        let mut args = vec![
            "run".to_owned(),
            "--rpc".to_owned(),
            "-H".to_owned(),
            review_config.hats_file.clone(),
        ];
        if let Some(backend) = &review_config.backend {
            args.push("-b".to_owned());
            args.push(backend.clone());
        }

        let command = self.format_command(&args);
        let mut cmd = Command::new(&self.config.command);
        for arg in &args {
            cmd.arg(arg);
        }

        cmd.current_dir(workspace.as_ref());
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn().context(WorkspaceIoSnafu {
            message: format!(
                "failed to spawn `{command}` in {}",
                workspace.as_ref().display()
            ),
        })?;
        let stdin = child.stdin.take();

        Ok(AgentHandle {
            child,
            stdin,
            started_at: Instant::now(),
        })
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
            .context(WorkspaceIoSnafu {
                message: format!("failed to write prompt {}", prompt_path.display()),
            })?;

        let args = self.config.command_args();
        let command = self.format_command(&args);
        let mut cmd = Command::new(&self.config.command);
        for arg in &args {
            cmd.arg(arg);
        }

        cmd.current_dir(workspace.as_ref());
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn().context(WorkspaceIoSnafu {
            message: format!(
                "failed to spawn `{command}` in {}",
                workspace.as_ref().display()
            ),
        })?;
        let stdin = child.stdin.take();

        Ok(AgentHandle {
            child,
            stdin,
            started_at: Instant::now(),
        })
    }
}

/// Convert a title string into a URL-safe slug for branch names.
fn slug(title: &str) -> String {
    title
        .chars()
        .filter_map(|c| {
            if c.is_alphanumeric() {
                Some(c.to_ascii_lowercase())
            } else if c == ' ' || c == '-' {
                Some('-')
            } else {
                None
            }
        })
        .take(40)
        .collect()
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn merge_core_config_overrides_generated_values() {
        let generated = "agent:\n  backend: local\n";
        let core = "agent:\n  backend: codex\n";
        let merged = merge_core_config(generated, core).expect("merge should succeed");
        assert!(merged.contains("backend: codex"));
    }

    #[tokio::test]
    async fn missing_core_config_keeps_generated_ralph_config() {
        let workspace = TempDir::new().expect("tempdir should exist");
        let generated_path = workspace.path().join("ralph.yml");
        tokio::fs::write(&generated_path, "agent:\n  backend: local\n")
            .await
            .expect("generated config should be written");

        let agent = RalphAgent::new(AgentConfig::default());
        agent
            .merge_core_config_if_present(workspace.path())
            .await
            .expect("missing core config should be ignored");

        let merged = tokio::fs::read_to_string(&generated_path)
            .await
            .expect("generated config should still be readable");
        assert!(merged.contains("backend: local"));
    }
}
