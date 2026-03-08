use std::time::Duration;

use bon::Builder;
use serde::{Deserialize, Serialize};

fn default_active_labels() -> Vec<String> {
    vec!["symphony:ready".to_owned()]
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

    /// Repository configurations.
    pub repos: Vec<RepoConfig>,
}

#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct RepoConfig {
    /// Display name for the repository (e.g. "rararulab/rara").
    pub name: String,

    /// Remote URL of the repository.
    pub url: String,

    /// Labels that mark an issue as ready for symphony.
    #[serde(default = "default_active_labels")]
    pub active_labels: Vec<String>,
}
