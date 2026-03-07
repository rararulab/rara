use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::header::{ACCEPT, AUTHORIZATION, USER_AGENT};
use serde::Deserialize;
use snafu::ensure;

use crate::config::RepoConfig;
use crate::error::{GitHubSnafu, Result};
use crate::event::{IssueState, TrackedIssue};

/// Trait for fetching issues from a remote tracker.
#[async_trait]
pub trait IssueTracker: Send + Sync {
    /// Fetch all active issues across configured repositories.
    async fn fetch_active_issues(&self) -> Result<Vec<TrackedIssue>>;

    /// Fetch the current state of a single issue.
    async fn fetch_issue_state(&self, repo: &str, number: u64) -> Result<IssueState>;
}

/// GitHub-backed issue tracker using the REST API.
pub struct GitHubIssueTracker {
    repos: Vec<RepoConfig>,
    client: reqwest::Client,
    token: Option<String>,
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

        let resp = self.get(&url).send().await.map_err(|e| {
            GitHubSnafu {
                message: format!("request failed for {}: {e}", repo.name),
            }
            .build()
        })?;

        ensure!(
            resp.status().is_success(),
            GitHubSnafu {
                message: format!(
                    "GitHub API returned {} for {}",
                    resp.status(),
                    repo.name
                ),
            }
        );

        let items: Vec<GitHubIssue> = resp.json().await.map_err(|e| {
            GitHubSnafu {
                message: format!("failed to parse response for {}: {e}", repo.name),
            }
            .build()
        })?;

        let issues = items
            .into_iter()
            .filter(|item| item.pull_request.is_none())
            .map(|item| {
                let labels: Vec<String> =
                    item.labels.into_iter().map(|l| l.name).collect();
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

    async fn fetch_issue_state(&self, repo: &str, number: u64) -> Result<IssueState> {
        let (owner, name) = parse_repo_slug(repo);
        let url = format!(
            "https://api.github.com/repos/{owner}/{name}/issues/{number}"
        );

        let resp = self.get(&url).send().await.map_err(|e| {
            GitHubSnafu {
                message: format!("request failed for {repo}#{number}: {e}"),
            }
            .build()
        })?;

        ensure!(
            resp.status().is_success(),
            GitHubSnafu {
                message: format!(
                    "GitHub API returned {} for {repo}#{number}",
                    resp.status()
                ),
            }
        );

        let item: GitHubIssue = resp.json().await.map_err(|e| {
            GitHubSnafu {
                message: format!("failed to parse issue {repo}#{number}: {e}"),
            }
            .build()
        })?;

        if item.state == "closed" {
            Ok(IssueState::Terminal)
        } else {
            Ok(IssueState::Active)
        }
    }
}

/// Parse an `"owner/repo"` slug into `(owner, repo)`.
///
/// If no slash is present, treats the whole string as repo with empty owner.
fn parse_repo_slug(slug: &str) -> (&str, &str) {
    slug.split_once('/').unwrap_or(("", slug))
}

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
    number: u64,
    title: String,
    body: Option<String>,
    state: String,
    labels: Vec<GitHubLabel>,
    created_at: DateTime<Utc>,
    pull_request: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct GitHubLabel {
    name: String,
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
        assert_eq!(
            derive_priority(&[s("priority:high"), s("priority:low")]),
            2
        );
    }

    #[test]
    fn sort_issues_priority_then_age() {
        let t1 = dt("2024-01-01T00:00:00Z");
        let t2 = dt("2024-01-02T00:00:00Z");
        let t3 = dt("2024-01-03T00:00:00Z");

        let mut issues = vec![
            issue("c", 3, u32::MAX, t1), // no priority, oldest
            issue("a", 1, 1, t2),         // priority 1, newer
            issue("b", 2, 1, t1),         // priority 1, older
            issue("d", 4, u32::MAX, t2),  // no priority, newer
            issue("e", 5, 2, t3),         // priority 2
        ];

        sort_issues(&mut issues);

        let ids: Vec<&str> = issues.iter().map(|i| i.id.as_str()).collect();
        // priority 1 (older first) → priority 2 → no priority (older first)
        assert_eq!(ids, vec!["b", "a", "e", "c", "d"]);
    }

    fn s(v: &str) -> String {
        v.to_owned()
    }

    fn dt(rfc3339: &str) -> DateTime<Utc> {
        rfc3339.parse().unwrap()
    }

    fn issue(
        id: &str,
        number: u64,
        priority: u32,
        created_at: DateTime<Utc>,
    ) -> TrackedIssue {
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
}
