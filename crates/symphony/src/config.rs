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

use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use bon::Builder;
use serde::{Deserialize, Serialize};

pub(crate) fn default_active_labels() -> Vec<String> {
    vec!["symphony:ready".to_owned()]
}

fn default_workflow_file() -> String {
    "WORKFLOW.md".to_owned()
}

fn default_command() -> String {
    "ralph".to_owned()
}

fn default_backend() -> String {
    "codex".to_owned()
}

fn default_assign_prefix() -> String {
    "ralph:".to_owned()
}

fn default_core_config_file() -> PathBuf {
    PathBuf::from("ralph.core.yml")
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
    vec!["Todo".to_owned()]
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

fn default_started_issue_state() -> String {
    "In Progress".to_owned()
}

fn default_completed_issue_state() -> String {
    "ToVerify".to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TrackerConfig {
    Github {
        /// GitHub personal access token. Supports `$ENV_VAR` syntax.
        api_key: Option<String>,

        /// Tracker state applied once Ralph starts successfully.
        #[serde(default = "default_started_issue_state")]
        started_issue_state: String,

        /// Tracker state applied once Ralph completes successfully.
        #[serde(default = "default_completed_issue_state")]
        completed_issue_state: String,
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

        /// Tracker state applied once Ralph starts successfully.
        #[serde(default = "default_started_issue_state")]
        started_issue_state: String,

        /// Tracker state applied once Ralph completes successfully.
        #[serde(default = "default_completed_issue_state")]
        completed_issue_state: String,
    },
}

impl TrackerConfig {
    #[must_use]
    pub fn active_states(&self) -> &[String] {
        match self {
            Self::Linear { active_states, .. } => active_states,
            Self::Github { .. } => &[],
        }
    }

    #[must_use]
    pub fn started_issue_state(&self) -> &str {
        match self {
            Self::Github {
                started_issue_state,
                ..
            }
            | Self::Linear {
                started_issue_state,
                ..
            } => started_issue_state,
        }
    }

    #[must_use]
    pub fn completed_issue_state(&self) -> &str {
        match self {
            Self::Github {
                completed_issue_state,
                ..
            }
            | Self::Linear {
                completed_issue_state,
                ..
            } => completed_issue_state,
        }
    }
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
    #[builder(default = default_command())]
    pub command: String,

    /// Backend passed to `ralph init --backend`.
    #[serde(default = "default_backend")]
    #[builder(default = default_backend())]
    pub backend: String,

    /// Prefix used to interpret issue assignment values as Ralph backends.
    #[serde(default = "default_assign_prefix")]
    #[builder(default = default_assign_prefix())]
    pub assign_prefix: String,

    /// Repository-root Ralph core config layered onto generated `ralph.yml`.
    #[serde(default = "default_core_config_file")]
    #[builder(default = default_core_config_file())]
    pub core_config_file: PathBuf,

    /// Extra args to pass to `ralph run`.
    #[serde(default)]
    #[builder(default)]
    pub extra_args: Vec<String>,

    /// Timeout for a single agent run.
    #[serde(
        default,
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize",
        skip_serializing_if = "Option::is_none"
    )]
    pub run_timeout: Option<Duration>,

    /// Named backend configurations addressable via `assign` values like
    /// `ralph:docker`.
    #[serde(default)]
    #[builder(default)]
    pub backends: BTreeMap<String, BackendConfig>,
}

#[derive(Debug, Clone, Builder, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackendConfig {
    /// The command to invoke ralph.
    #[serde(default = "default_command")]
    #[builder(default = default_command())]
    pub command: String,

    /// Backend passed to `ralph init --backend`.
    #[serde(default = "default_backend")]
    #[builder(default = default_backend())]
    pub backend: String,

    /// Repository-root Ralph core config layered onto generated `ralph.yml`.
    #[serde(default = "default_core_config_file")]
    #[builder(default = default_core_config_file())]
    pub core_config_file: PathBuf,

    /// Extra args to pass to `ralph run`.
    #[serde(default)]
    #[builder(default)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendSelection {
    Default,
    Assigned { key: String, raw: String },
    Ignored { raw: String },
    Invalid { key: String, raw: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAgentConfig {
    pub selection: BackendSelection,
    pub config: BackendConfig,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            command: default_command(),
            backend: default_backend(),
            assign_prefix: default_assign_prefix(),
            core_config_file: default_core_config_file(),
            extra_args: Vec::new(),
            run_timeout: None,
            backends: BTreeMap::new(),
        }
    }
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self {
            command: default_command(),
            backend: default_backend(),
            core_config_file: default_core_config_file(),
            extra_args: Vec::new(),
            run_timeout: None,
        }
    }
}

impl AgentConfig {
    #[must_use]
    pub fn default_backend_config(&self) -> BackendConfig {
        BackendConfig {
            command: self.command.clone(),
            backend: self.backend.clone(),
            core_config_file: self.core_config_file.clone(),
            extra_args: self.extra_args.clone(),
            run_timeout: self.run_timeout,
        }
    }

    #[must_use]
    pub fn assign_backend_key(&self, assign: Option<&str>) -> Option<String> {
        let assign = assign?.trim();
        if assign.is_empty() {
            return None;
        }

        let prefix = self.assign_prefix.trim();
        let key = assign.strip_prefix(prefix)?.trim();
        if key.is_empty() {
            return None;
        }

        Some(key.to_owned())
    }

    #[must_use]
    pub fn resolve_for_assign(&self, assign: Option<&str>) -> ResolvedAgentConfig {
        let default_config = self.default_backend_config();
        let raw = assign.map(str::trim).filter(|value| !value.is_empty());
        let Some(raw) = raw else {
            return ResolvedAgentConfig {
                selection: BackendSelection::Default,
                config: default_config,
            };
        };

        let Some(key) = self.assign_backend_key(Some(raw)) else {
            return ResolvedAgentConfig {
                selection: BackendSelection::Ignored {
                    raw: raw.to_owned(),
                },
                config: default_config,
            };
        };

        match self.backends.get(&key) {
            Some(config) => ResolvedAgentConfig {
                selection: BackendSelection::Assigned {
                    key,
                    raw: raw.to_owned(),
                },
                config: config.clone(),
            },
            None => ResolvedAgentConfig {
                selection: BackendSelection::Invalid {
                    key,
                    raw: raw.to_owned(),
                },
                config: default_config,
            },
        }
    }

    #[must_use]
    pub fn init_args(&self) -> Vec<String> {
        self.default_backend_config().init_args()
    }

    #[must_use]
    pub fn command_args(&self) -> Vec<String> {
        self.default_backend_config().command_args()
    }

    #[must_use]
    pub fn doctor_args(&self) -> Vec<String> {
        self.default_backend_config().doctor_args()
    }
}

impl BackendConfig {
    #[must_use]
    pub fn init_args(&self) -> Vec<String> {
        vec![
            "init".to_owned(),
            "--force".to_owned(),
            "--backend".to_owned(),
            self.backend.clone(),
            "-c".to_owned(),
            self.core_config_file.display().to_string(),
        ]
    }

    #[must_use]
    pub fn command_args(&self) -> Vec<String> {
        let mut args = vec!["run".to_owned()];
        let explicit_mode = self
            .extra_args
            .iter()
            .any(|arg| matches!(arg.as_str(), "--autonomous" | "--no-tui"));
        if !explicit_mode {
            args.push("--autonomous".to_owned());
        }
        args.extend(self.extra_args.iter().cloned());
        args
    }

    #[must_use]
    pub fn doctor_args(&self) -> Vec<String> {
        vec!["doctor".to_owned()]
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

pub(crate) fn default_repo_url(repo_name: &str) -> String {
    format!("git@github.com:{repo_name}.git")
}

pub(crate) fn default_repo_checkout_root(repo_name: &str) -> PathBuf {
    rara_paths::config_dir()
        .join("ralpha/repos")
        .join(repo_name)
}

fn default_workspace_root(repo_name: &str) -> PathBuf {
    rara_paths::config_dir()
        .join("ralpha/worktress")
        .join(repo_name)
        .join("worktrees")
}
