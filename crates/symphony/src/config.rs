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

use std::{path::PathBuf, time::Duration};

use bon::Builder;
use serde::{Deserialize, Serialize};

fn default_active_labels() -> Vec<String> { vec!["symphony:ready".to_owned()] }

fn default_workflow_file() -> String { "WORKFLOW.md".to_owned() }

fn default_command() -> String { "ralph".to_owned() }

fn default_max_concurrent_agents() -> usize { 2 }

fn default_stall_timeout() -> Duration { Duration::from_secs(30 * 60) }

fn default_max_retry_backoff() -> Duration { Duration::from_secs(60 * 60) }

fn default_active_states() -> Vec<String> { vec!["Todo".to_owned(), "In Progress".to_owned()] }

fn default_terminal_states() -> Vec<String> {
    vec![
        "Done".to_owned(),
        "Closed".to_owned(),
        "Cancelled".to_owned(),
        "Canceled".to_owned(),
        "Duplicate".to_owned(),
    ]
}

fn default_repo_label_prefix() -> String { "repo:".to_owned() }

fn default_linear_endpoint() -> String { "https://api.linear.app/graphql".to_owned() }

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
        serialize_with = "humantime_serde::serialize"
    )]
    pub poll_interval: Duration,

    /// Maximum number of concurrent coding agents across all repos.
    #[serde(default = "default_max_concurrent_agents")]
    pub max_concurrent_agents: usize,

    /// How long before an agent is considered stalled.
    #[serde(
        default = "default_stall_timeout",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    pub stall_timeout: Duration,

    /// Maximum backoff duration for retries.
    #[serde(
        default = "default_max_retry_backoff",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
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
    /// The command to invoke ralph.
    #[serde(default = "default_command")]
    pub command: String,

    /// Optional path to a ralph config file.
    #[serde(default)]
    pub config_file: Option<PathBuf>,

    /// Extra args to pass to `ralph run`.
    #[serde(default)]
    pub extra_args: Vec<String>,

    /// Timeout for a single agent run.
    #[serde(
        default,
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize",
        skip_serializing_if = "Option::is_none"
    )]
    pub run_timeout: Option<Duration>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            command:     default_command(),
            config_file: None,
            extra_args:  Vec::new(),
            run_timeout: None,
        }
    }
}

impl AgentConfig {
    #[must_use]
    pub fn command_args(&self) -> Vec<String> {
        let mut args = vec!["run".to_owned()];
        if let Some(path) = &self.config_file {
            args.push("-c".to_owned());
            args.push(path.display().to_string());
        }
        args.push("--no-tui".to_owned());
        args.extend(self.extra_args.iter().cloned());
        args
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
