use std::path::PathBuf;
use std::time::Duration;

use bon::Builder;
use serde::{Deserialize, Serialize};

fn default_workflow_file() -> String {
    "WORKFLOW.md".to_owned()
}

fn default_command() -> String {
    "claude".to_owned()
}

fn default_active_labels() -> Vec<String> {
    vec!["symphony:ready".to_owned()]
}

fn default_max_concurrent_agents() -> usize {
    2
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

        /// Linear project slug (slugId).
        project_slug: String,

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
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize",
    )]
    pub stall_timeout: Duration,

    /// Maximum backoff duration for retries.
    #[serde(
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize",
    )]
    pub max_retry_backoff: Duration,

    /// Path to the workflow file template.
    #[serde(default = "default_workflow_file")]
    pub workflow_file: String,

    /// Agent configuration.
    pub agent: AgentConfig,

    /// Repository configurations.
    pub repos: Vec<RepoConfig>,
}

#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct AgentConfig {
    /// The command to invoke the coding agent.
    #[serde(default = "default_command")]
    pub command: String,

    /// Arguments to pass to the agent command.
    #[serde(default)]
    pub args: Vec<String>,

    /// Tools the agent is allowed to use.
    #[serde(default)]
    pub allowed_tools: Vec<String>,

    /// Timeout for a single agent turn.
    #[serde(
        default,
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize",
        skip_serializing_if = "Option::is_none",
    )]
    pub turn_timeout: Option<Duration>,
}

#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct RepoConfig {
    /// Display name for the repository.
    pub name: String,

    /// Remote URL of the repository.
    pub url: String,

    /// Local path to the repository checkout.
    pub repo_path: PathBuf,

    /// Root directory for worktrees.
    pub workspace_root: PathBuf,

    /// Labels that mark an issue as ready for symphony.
    #[serde(default = "default_active_labels")]
    pub active_labels: Vec<String>,

    /// Per-repo override for max concurrent agents.
    pub max_concurrent_agents: Option<usize>,

    /// Per-repo override for the workflow file.
    pub workflow_file: Option<String>,

    /// Hook scripts to run at various lifecycle points.
    #[serde(default)]
    pub hooks: HooksConfig,
}

#[derive(Debug, Clone, Default, Builder, Serialize, Deserialize)]
pub struct HooksConfig {
    /// Script to run after worktree creation.
    pub after_create: Option<String>,

    /// Script to run before the agent starts.
    pub before_run: Option<String>,

    /// Script to run after the agent finishes.
    pub after_run: Option<String>,

    /// Script to run before worktree removal.
    pub before_remove: Option<String>,
}
