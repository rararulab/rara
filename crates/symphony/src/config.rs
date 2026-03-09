use std::path::PathBuf;
use std::time::Duration;

use bon::Builder;
use serde::{Deserialize, Serialize};

fn default_active_labels() -> Vec<String> {
    vec!["symphony:ready".to_owned()]
}

fn default_workflow_file() -> String {
    "WORKFLOW.md".to_owned()
}

fn default_command() -> String {
    "ralph".to_owned()
}

/// Execution backend for the coding agent subprocess.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentBackend {
    /// Ralph CLI (`ralph run`) — the default backend.
    #[default]
    Ralph,
    /// OpenAI Codex CLI (`codex --approval-mode full-auto`).
    Codex,
}

fn default_max_concurrent_agents() -> usize {
    2
}

fn default_stall_timeout() -> Duration {
    Duration::from_secs(30 * 60)
}

fn default_max_retry_backoff() -> Duration {
    Duration::from_secs(60 * 60)
}

fn default_active_states() -> Vec<String> {
    vec!["Todo".to_owned(), "In Progress".to_owned()]
}

fn default_terminal_states() -> Vec<String> {
    vec![
        "Done".to_owned(),
        "Closed".to_owned(),
        "Cancelled".to_owned(),
        "Canceled".to_owned(),
        "Duplicate".to_owned(),
    ]
}

fn default_repo_label_prefix() -> String {
    "repo:".to_owned()
}

fn default_linear_endpoint() -> String {
    "https://api.linear.app/graphql".to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TrackerConfig {
    Github {
        /// GitHub personal access token. Supports `$ENV_VAR` syntax.
        api_key: Option<String>,
    },
    Linear {
        /// Linear API key. Supports `$ENV_VAR` syntax.
        api_key: String,

        /// Linear team key (e.g. "RAR", "ENG"). Required.
        team_key: String,

        /// Linear project slug (optional, for further filtering within a team).
        #[serde(default)]
        project_slug: Option<String>,

        /// GraphQL endpoint override.
        #[serde(default = "default_linear_endpoint")]
        endpoint: String,

        /// Issue states that trigger dispatch.
        #[serde(default = "default_active_states")]
        active_states: Vec<String>,

        /// Issue states considered terminal.
        #[serde(default = "default_terminal_states")]
        terminal_states: Vec<String>,

        /// Label prefix for repo mapping.
        #[serde(default = "default_repo_label_prefix")]
        repo_label_prefix: String,
    },
}

#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct SymphonyConfig {
    /// Whether the symphony system is enabled.
    #[serde(default)]
    pub enabled: bool,

    /// Issue tracker configuration. None defaults to GitHub with env token.
    pub tracker: Option<TrackerConfig>,

    /// How often to poll for new issues.
    #[serde(
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize",
    )]
    pub poll_interval: Duration,

    /// Maximum number of concurrent coding agents across all repos.
    #[serde(default = "default_max_concurrent_agents")]
    pub max_concurrent_agents: usize,

    /// How long before an agent is considered stalled.
    #[serde(
        default = "default_stall_timeout",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize",
    )]
    pub stall_timeout: Duration,

    /// Maximum backoff duration for retries.
    #[serde(
        default = "default_max_retry_backoff",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize",
    )]
    pub max_retry_backoff: Duration,

    /// Default workflow file name to read from a worktree.
    #[serde(default = "default_workflow_file")]
    pub workflow_file: String,

    /// Agent execution configuration.
    #[serde(default)]
    pub agent: AgentConfig,

    /// Repository configurations.
    pub repos: Vec<RepoConfig>,
}

#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Which execution backend to use.
    #[serde(default)]
    #[builder(default)]
    pub backend: AgentBackend,

    /// Override the command binary. When absent, derived from `backend`
    /// (`"ralph"` for Ralph, `"codex"` for Codex).
    #[serde(default)]
    pub command: Option<String>,

    /// Optional path to a ralph config file (Ralph backend only).
    #[serde(default)]
    pub config_file: Option<PathBuf>,

    /// Extra args appended to the generated command line.
    #[serde(default)]
    #[builder(default)]
    pub extra_args: Vec<String>,

    /// Timeout for a single agent run.
    #[serde(
        default,
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize",
        skip_serializing_if = "Option::is_none",
    )]
    pub run_timeout: Option<Duration>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            backend: AgentBackend::default(),
            command: None,
            config_file: None,
            extra_args: Vec::new(),
            run_timeout: None,
        }
    }
}

impl AgentConfig {
    /// The binary to invoke, derived from `backend` unless explicitly overridden.
    #[must_use]
    pub fn effective_command(&self) -> &str {
        self.command.as_deref().unwrap_or(match self.backend {
            AgentBackend::Ralph => "ralph",
            AgentBackend::Codex => "codex",
        })
    }

    /// Build the argument list for the agent subprocess.
    ///
    /// `prompt` is required for the Codex backend (passed as a positional arg)
    /// and ignored for Ralph (which reads `PROMPT.md` from the working directory).
    #[must_use]
    pub fn command_args(&self, prompt: Option<&str>) -> Vec<String> {
        match self.backend {
            AgentBackend::Ralph => {
                let mut args = vec!["run".to_owned()];
                if let Some(path) = &self.config_file {
                    args.push("-c".to_owned());
                    args.push(path.display().to_string());
                }
                args.push("--no-tui".to_owned());
                args.extend(self.extra_args.iter().cloned());
                args
            }
            AgentBackend::Codex => {
                let mut args = vec![
                    "--quiet".to_owned(),
                    "--approval-mode".to_owned(),
                    "full-auto".to_owned(),
                ];
                args.extend(self.extra_args.iter().cloned());
                if let Some(p) = prompt {
                    args.push(p.to_owned());
                }
                args
            }
        }
    }
}

#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct RepoConfig {
    /// Display name for the repository (e.g. "rararulab/rara").
    pub name: String,

    /// Remote URL of the repository.
    pub url: String,

    /// Optional local path to the repository checkout.
    #[serde(default)]
    pub repo_path: Option<PathBuf>,

    /// Optional root directory for issue worktrees.
    #[serde(default)]
    pub workspace_root: Option<PathBuf>,

    /// Labels that mark an issue as ready for symphony.
    #[serde(default = "default_active_labels")]
    pub active_labels: Vec<String>,

    /// Per-repo override for max concurrent agents.
    #[serde(default)]
    pub max_concurrent_agents: Option<usize>,

    /// Per-repo override for the workflow file.
    #[serde(default)]
    pub workflow_file: Option<String>,
}

impl RepoConfig {
    pub fn effective_workspace_root(&self) -> Option<PathBuf> {
        self.workspace_root
            .clone()
            .or_else(|| Some(default_workspace_root(&self.name)))
    }
}

fn default_workspace_root(repo_name: &str) -> PathBuf {
    rara_paths::config_dir()
        .join("ralpha/worktress")
        .join(repo_name)
        .join("worktrees")
}
