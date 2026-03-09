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

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::header::{ACCEPT, AUTHORIZATION, USER_AGENT};
use serde::Deserialize;
use snafu::{ResultExt, ensure};

use crate::{
    config::RepoConfig,
    error::{GitHubRequestSnafu, GitHubStatusSnafu, LinearSnafu, Result},
};

/// Represents the lifecycle state of a tracked issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueState {
    /// Issue is actively being worked on or waiting for an agent.
    Active,
    /// Issue has reached a terminal state (closed, merged, etc.).
    Terminal,
}

/// An issue that symphony is tracking.
#[derive(Debug, Clone)]
pub struct TrackedIssue {
    /// Unique identifier (owner/repo#number).
    pub id:         String,
    /// Human-readable identifier. GitHub: "42", Linear: "RAR-42".
    pub identifier: String,
    /// Repository name (owner/repo).
    pub repo:       String,
    /// Issue number.
    pub number:     u64,
    /// Issue title.
    pub title:      String,
    /// Issue body/description.
    pub body:       Option<String>,
    /// Labels attached to the issue.
    pub labels:     Vec<String>,
    /// Priority (lower = higher priority).
    pub priority:   u32,
    /// Current lifecycle state.
    pub state:      IssueState,
    /// When the issue was created.
    pub created_at: DateTime<Utc>,
}

/// Trait for fetching issues from a remote tracker.
#[async_trait]
pub trait IssueTracker: Send + Sync {
    /// Fetch all active issues across configured repositories.
    async fn fetch_active_issues(&self) -> Result<Vec<TrackedIssue>>;

    /// Fetch the current state of a single issue.
    async fn fetch_issue_state(&self, issue: &TrackedIssue) -> Result<IssueState>;

    /// Transition an issue to a new state (e.g. "In Progress").
    ///
    /// Implementations should be best-effort — a failure here should not
    /// block agent dispatch.
    async fn transition_issue(&self, issue: &TrackedIssue, state_name: &str) -> Result<()>;
}

/// GitHub-backed issue tracker using the REST API.
pub struct GitHubIssueTracker {
    repos:  Vec<RepoConfig>,
    client: reqwest::Client,
    token:  Option<String>,
}

impl GitHubIssueTracker {
    /// Create a new GitHub issue tracker.
    ///
    /// # Arguments
    /// * `repos` — repository configurations to poll
    /// * `token` — optional GitHub personal access token for authentication
    #[must_use]
    pub fn new(repos: Vec<RepoConfig>, token: Option<String>) -> Self {
        Self {
            repos,
            client: reqwest::Client::new(),
            token,
        }
    }

    /// Build a GET request with common headers and optional auth.
    fn get(&self, url: &str) -> reqwest::RequestBuilder {
        let mut req = self
            .client
            .get(url)
            .header(USER_AGENT, "rara-symphony")
            .header(ACCEPT, "application/vnd.github+json");

        if let Some(token) = &self.token {
            req = req.header(AUTHORIZATION, format!("Bearer {token}"));
        }

        req
    }

    /// Fetch open issues for a single repository that match its active labels.
    async fn fetch_repo_issues(&self, repo: &RepoConfig) -> Result<Vec<TrackedIssue>> {
        let (owner, name) = parse_repo_slug(&repo.name);
        let labels = repo.active_labels.join(",");

        let url = format!(
            "https://api.github.com/repos/{owner}/{name}/issues\
             ?state=open&labels={labels}&per_page=100"
        );

        let resp = self.get(&url).send().await.context(GitHubRequestSnafu {
            repo: repo.name.clone(),
        })?;

        ensure!(
            resp.status().is_success(),
            GitHubStatusSnafu {
                repo:   repo.name.clone(),
                status: resp.status(),
            }
        );

        let items: Vec<GitHubIssue> = resp.json().await.context(GitHubRequestSnafu {
            repo: repo.name.clone(),
        })?;

        let issues = items
            .into_iter()
            .filter(|item| item.pull_request.is_none())
            .map(|item| {
                let labels: Vec<String> = item.labels.into_iter().map(|l| l.name).collect();
                let priority = derive_priority(&labels);
                TrackedIssue {
                    id: format!("{}#{}", repo.name, item.number),
                    identifier: item.number.to_string(),
                    repo: repo.name.clone(),
                    number: item.number,
                    title: item.title,
                    body: item.body,
                    labels,
                    priority,
                    state: IssueState::Active,
                    created_at: item.created_at,
                }
            })
            .collect();

        Ok(issues)
    }
}

#[async_trait]
impl IssueTracker for GitHubIssueTracker {
    async fn fetch_active_issues(&self) -> Result<Vec<TrackedIssue>> {
        let mut all_issues = Vec::new();

        for repo in &self.repos {
            match self.fetch_repo_issues(repo).await {
                Ok(issues) => all_issues.extend(issues),
                Err(e) => {
                    tracing::warn!(
                        repo = %repo.name,
                        error = %e,
                        "failed to fetch issues, skipping"
                    );
                }
            }
        }

        sort_issues(&mut all_issues);
        Ok(all_issues)
    }

    async fn fetch_issue_state(&self, issue: &TrackedIssue) -> Result<IssueState> {
        let (owner, name) = parse_repo_slug(&issue.repo);
        let number = issue.number;
        let url = format!("https://api.github.com/repos/{owner}/{name}/issues/{number}");

        let repo = &issue.repo;
        let resp = self
            .get(&url)
            .send()
            .await
            .context(GitHubRequestSnafu { repo: repo.clone() })?;

        ensure!(
            resp.status().is_success(),
            GitHubStatusSnafu {
                repo:   repo.clone(),
                status: resp.status(),
            }
        );

        let item: GitHubIssue = resp
            .json()
            .await
            .context(GitHubRequestSnafu { repo: repo.clone() })?;

        if item.state == "closed" {
            Ok(IssueState::Terminal)
        } else {
            Ok(IssueState::Active)
        }
    }

    async fn transition_issue(&self, issue: &TrackedIssue, state_name: &str) -> Result<()> {
        // GitHub doesn't have workflow states — log and skip.
        tracing::debug!(
            issue_id = %issue.id,
            state = state_name,
            "github: state transitions not supported, skipping"
        );
        Ok(())
    }
}

/// Parse an `"owner/repo"` slug into `(owner, repo)`.
///
/// If no slash is present, treats the whole string as repo with empty owner.
fn parse_repo_slug(slug: &str) -> (&str, &str) { slug.split_once('/').unwrap_or(("", slug)) }

/// Derive a numeric priority from issue labels.
///
/// Looks for labels matching `priority:<value>` where value is:
/// - A number (e.g. `priority:1`)
/// - `critical` → 1
/// - `high` → 2
/// - `medium` → 3
/// - `low` → 4
///
/// Returns `u32::MAX` if no priority label is found.
pub fn derive_priority(labels: &[String]) -> u32 {
    for label in labels {
        if let Some(value) = label.strip_prefix("priority:") {
            if let Ok(n) = value.parse::<u32>() {
                return n;
            }
            return match value {
                "critical" => 1,
                "high" => 2,
                "medium" => 3,
                "low" => 4,
                _ => continue,
            };
        }
    }
    u32::MAX
}

/// Sort issues by: priority ascending (`u32::MAX` = last), then `created_at`
/// oldest first, then issue number ascending.
fn sort_issues(issues: &mut [TrackedIssue]) {
    issues.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then_with(|| a.created_at.cmp(&b.created_at))
            .then_with(|| a.number.cmp(&b.number))
    });
}

// ── GitHub API response types ────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct GitHubIssue {
    number:       u64,
    title:        String,
    body:         Option<String>,
    state:        String,
    labels:       Vec<GitHubLabel>,
    created_at:   DateTime<Utc>,
    pull_request: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct GitHubLabel {
    name: String,
}

// ── Linear-backed issue tracker ──────────────────────────────────────

/// Linear-backed issue tracker using the GraphQL API.
pub struct LinearIssueTracker {
    client:            lineark_sdk::Client,
    team_key:          String,
    project_slug:      Option<String>,
    active_states:     Vec<String>,
    terminal_states:   Vec<String>,
    repo_label_prefix: String,
    repos:             Vec<String>,
}

impl LinearIssueTracker {
    /// Create a new Linear issue tracker.
    pub fn new(
        api_key: &str,
        endpoint: &str,
        team_key: String,
        project_slug: Option<String>,
        active_states: Vec<String>,
        terminal_states: Vec<String>,
        repo_label_prefix: String,
        repos: Vec<String>,
    ) -> Result<Self> {
        let mut client = lineark_sdk::Client::from_token(api_key).context(LinearSnafu {
            message: "failed to create client",
        })?;
        client.set_base_url(endpoint.to_owned());
        Ok(Self {
            client,
            team_key,
            project_slug,
            active_states,
            terminal_states,
            repo_label_prefix,
            repos,
        })
    }

    /// Extract repository name from issue labels using the configured prefix.
    fn extract_repo(&self, labels: &[String]) -> Option<String> {
        let prefix = self.repo_label_prefix.to_lowercase();
        for label in labels {
            let lower = label.to_lowercase();
            if let Some(repo) = lower.strip_prefix(&prefix) {
                if self.repos.iter().any(|r| r == repo) {
                    return Some(repo.to_owned());
                }
            }
        }
        None
    }

    /// Parse the numeric issue number from a Linear identifier like `"RAR-42"`.
    fn parse_number(identifier: &str) -> u64 {
        identifier
            .rsplit('-')
            .next()
            .and_then(|n| n.parse().ok())
            .unwrap_or(0)
    }

    /// Map Linear priority (0 = no priority) to our ordering (lower = higher
    /// priority).
    fn map_priority(linear_priority: u32) -> u32 {
        match linear_priority {
            0 => u32::MAX,
            n => n,
        }
    }
}

#[async_trait]
impl IssueTracker for LinearIssueTracker {
    async fn fetch_active_issues(&self) -> Result<Vec<TrackedIssue>> {
        tracing::debug!(
            team_key = %self.team_key,
            project_slug = ?self.project_slug,
            active_states = ?self.active_states,
            "linear: fetching candidate issues"
        );

        let mut all_issues = Vec::new();
        let mut after: Option<String> = None;

        loop {
            let (query, variables) = if let Some(ref slug) = self.project_slug {
                // Filter by team + project
                let q = r#"
                    query($teamKey: String!, $projectSlug: String!, $states: [String!]!, $first: Int!, $after: String) {
                        issues(
                            filter: {
                                team: { key: { eq: $teamKey } }
                                project: { slugId: { eq: $projectSlug } }
                                state: { name: { in: $states } }
                            }
                            first: $first
                            after: $after
                            orderBy: createdAt
                        ) {
                            nodes {
                                id identifier title description priority createdAt
                                state { name }
                                labels { nodes { name } }
                            }
                            pageInfo { hasNextPage endCursor }
                        }
                    }
                "#;
                let v = serde_json::json!({
                    "teamKey": self.team_key,
                    "projectSlug": slug,
                    "states": self.active_states,
                    "first": 50,
                    "after": after,
                });
                (q, v)
            } else {
                // Filter by team only
                let q = r#"
                    query($teamKey: String!, $states: [String!]!, $first: Int!, $after: String) {
                        issues(
                            filter: {
                                team: { key: { eq: $teamKey } }
                                state: { name: { in: $states } }
                            }
                            first: $first
                            after: $after
                            orderBy: createdAt
                        ) {
                            nodes {
                                id identifier title description priority createdAt
                                state { name }
                                labels { nodes { name } }
                            }
                            pageInfo { hasNextPage endCursor }
                        }
                    }
                "#;
                let v = serde_json::json!({
                    "teamKey": self.team_key,
                    "states": self.active_states,
                    "first": 50,
                    "after": after,
                });
                (q, v)
            };

            let conn = self
                .client
                .execute_connection::<serde_json::Value>(query, variables, "issues")
                .await
                .context(LinearSnafu {
                    message: "failed to fetch issues",
                })?;

            tracing::debug!(
                page_size = conn.nodes.len(),
                has_next_page = conn.page_info.has_next_page,
                "linear: fetched page of issues"
            );

            for node in &conn.nodes {
                // Extract label names.
                let labels: Vec<String> = node
                    .get("labels")
                    .and_then(|l| l.get("nodes"))
                    .and_then(|n| n.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.get("name").and_then(|n| n.as_str()))
                            .map(|s| s.to_lowercase())
                            .collect()
                    })
                    .unwrap_or_default();

                let repo = match self.extract_repo(&labels) {
                    Some(r) => r,
                    None => {
                        let ident = node
                            .get("identifier")
                            .and_then(|v| v.as_str())
                            .unwrap_or("?");
                        tracing::warn!(
                            identifier = ident,
                            labels = ?labels,
                            prefix = %self.repo_label_prefix,
                            configured_repos = ?self.repos,
                            "linear: no matching repo label, skipping issue"
                        );
                        continue;
                    }
                };

                let identifier = node
                    .get("identifier")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_owned();
                let number = Self::parse_number(&identifier);
                let linear_priority =
                    node.get("priority").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let priority = Self::map_priority(linear_priority);
                let created_at: DateTime<Utc> = node
                    .get("createdAt")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or_default();

                tracing::debug!(
                    identifier = %identifier,
                    title = %node.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                    repo = %repo,
                    priority = linear_priority,
                    labels = ?labels,
                    "linear: matched issue to repo"
                );

                all_issues.push(TrackedIssue {
                    id: node
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_owned(),
                    identifier,
                    repo,
                    number,
                    title: node
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_owned(),
                    body: node
                        .get("description")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_owned()),
                    labels,
                    priority,
                    state: IssueState::Active,
                    created_at,
                });
            }

            if conn.page_info.has_next_page {
                after = conn.page_info.end_cursor;
            } else {
                break;
            }
        }

        tracing::debug!(total = all_issues.len(), "linear: fetch complete");

        sort_issues(&mut all_issues);
        Ok(all_issues)
    }

    async fn fetch_issue_state(&self, issue: &TrackedIssue) -> Result<IssueState> {
        tracing::debug!(
            issue_id = %issue.id,
            identifier = %issue.identifier,
            "linear: checking issue state"
        );

        const QUERY: &str = r#"
            query($id: String!) {
                issue(id: $id) {
                    state { name }
                }
            }
        "#;

        let original_issue_id = &issue.id;
        let variables = serde_json::json!({ "id": original_issue_id });

        let issue: serde_json::Value = self
            .client
            .execute(QUERY, variables, "issue")
            .await
            .context(LinearSnafu {
                message: "failed to fetch issue state",
            })?;

        let state_name = issue
            .get("state")
            .and_then(|s| s.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or("");

        tracing::debug!(
            issue_id = %original_issue_id,
            state = state_name,
            "linear: issue state resolved"
        );

        let is_terminal = self
            .terminal_states
            .iter()
            .any(|ts| ts.eq_ignore_ascii_case(state_name));

        if is_terminal {
            Ok(IssueState::Terminal)
        } else {
            Ok(IssueState::Active)
        }
    }

    async fn transition_issue(&self, issue: &TrackedIssue, state_name: &str) -> Result<()> {
        // 1. Look up the target state ID by name for this team.
        let state_id = self.resolve_state_id(state_name).await?;

        // 2. Update the issue state.
        const MUTATION: &str = r#"
            mutation($id: String!, $stateId: String!) {
                issueUpdate(id: $id, input: { stateId: $stateId }) {
                    success
                }
            }
        "#;

        let variables = serde_json::json!({
            "id": issue.id,
            "stateId": state_id,
        });

        let result: serde_json::Value = self
            .client
            .execute(MUTATION, variables, "issueUpdate")
            .await
            .context(LinearSnafu {
                message: format!(
                    "failed to transition issue {} to '{state_name}'",
                    issue.identifier
                ),
            })?;

        let success = result
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if success {
            tracing::info!(
                issue_id = %issue.id,
                identifier = %issue.identifier,
                state = state_name,
                "linear: transitioned issue"
            );
        } else {
            tracing::warn!(
                issue_id = %issue.id,
                identifier = %issue.identifier,
                state = state_name,
                "linear: issueUpdate returned success=false"
            );
        }

        Ok(())
    }
}

impl LinearIssueTracker {
    /// Resolve a workflow state name (e.g. "In Progress") to its Linear state
    /// ID.
    async fn resolve_state_id(&self, state_name: &str) -> Result<String> {
        const QUERY: &str = r#"
            query($teamKey: String!, $stateName: String!) {
                workflowStates(
                    filter: {
                        team: { key: { eq: $teamKey } }
                        name: { eq: $stateName }
                    }
                    first: 1
                ) {
                    nodes { id name }
                }
            }
        "#;

        let variables = serde_json::json!({
            "teamKey": self.team_key,
            "stateName": state_name,
        });

        let conn = self
            .client
            .execute_connection::<serde_json::Value>(QUERY, variables, "workflowStates")
            .await
            .context(LinearSnafu {
                message: format!("failed to resolve state '{state_name}'"),
            })?;

        let state_id = conn
            .nodes
            .first()
            .and_then(|n| n.get("id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_owned());

        state_id.ok_or_else(|| {
            crate::error::ConfigSnafu {
                message: format!(
                    "workflow state '{state_name}' not found for team '{}'",
                    self.team_key
                ),
            }
            .build()
        })
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_from_labels() {
        // Numeric priority
        assert_eq!(derive_priority(&[s("priority:1")]), 1);
        assert_eq!(derive_priority(&[s("priority:42")]), 42);

        // Named priorities
        assert_eq!(derive_priority(&[s("priority:critical")]), 1);
        assert_eq!(derive_priority(&[s("priority:high")]), 2);
        assert_eq!(derive_priority(&[s("priority:medium")]), 3);
        assert_eq!(derive_priority(&[s("priority:low")]), 4);

        // No priority label → MAX
        assert_eq!(derive_priority(&[s("bug"), s("enhancement")]), u32::MAX);
        assert_eq!(derive_priority(&[]), u32::MAX);

        // Unknown priority value is skipped, falls through to MAX
        assert_eq!(derive_priority(&[s("priority:unknown")]), u32::MAX);

        // First matching priority wins
        assert_eq!(derive_priority(&[s("priority:high"), s("priority:low")]), 2);
    }

    #[test]
    fn sort_issues_priority_then_age() {
        let t1 = dt("2024-01-01T00:00:00Z");
        let t2 = dt("2024-01-02T00:00:00Z");
        let t3 = dt("2024-01-03T00:00:00Z");

        let mut issues = vec![
            issue("c", 3, u32::MAX, t1), // no priority, oldest
            issue("a", 1, 1, t2),        // priority 1, newer
            issue("b", 2, 1, t1),        // priority 1, older
            issue("d", 4, u32::MAX, t2), // no priority, newer
            issue("e", 5, 2, t3),        // priority 2
        ];

        sort_issues(&mut issues);

        let ids: Vec<&str> = issues.iter().map(|i| i.id.as_str()).collect();
        // priority 1 (older first) → priority 2 → no priority (older first)
        assert_eq!(ids, vec!["b", "a", "e", "c", "d"]);
    }

    fn s(v: &str) -> String { v.to_owned() }

    fn dt(rfc3339: &str) -> DateTime<Utc> { rfc3339.parse().unwrap() }

    fn issue(id: &str, number: u64, priority: u32, created_at: DateTime<Utc>) -> TrackedIssue {
        TrackedIssue {
            id: id.to_owned(),
            identifier: number.to_string(),
            repo: "test/repo".to_owned(),
            number,
            title: format!("Issue {number}"),
            body: None,
            labels: vec![],
            priority,
            state: IssueState::Active,
            created_at,
        }
    }

    #[test]
    fn linear_parse_number() {
        assert_eq!(LinearIssueTracker::parse_number("RAR-42"), 42);
        assert_eq!(LinearIssueTracker::parse_number("PROJ-1"), 1);
        assert_eq!(LinearIssueTracker::parse_number("X-0"), 0);
        assert_eq!(LinearIssueTracker::parse_number("invalid"), 0);
    }

    #[test]
    fn linear_map_priority() {
        assert_eq!(LinearIssueTracker::map_priority(0), u32::MAX);
        assert_eq!(LinearIssueTracker::map_priority(1), 1);
        assert_eq!(LinearIssueTracker::map_priority(2), 2);
        assert_eq!(LinearIssueTracker::map_priority(3), 3);
        assert_eq!(LinearIssueTracker::map_priority(4), 4);
    }
}
