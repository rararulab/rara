# Linear Issue Tracker Integration Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add Linear as an issue tracker source for rara-symphony, so users manage tasks in Linear and symphony auto-dispatches coding agents.

**Architecture:** Implement `LinearIssueTracker` (impl `IssueTracker` trait) using `lineark-sdk`. Add `TrackerConfig` to `SymphonyConfig` for global tracker selection. Linear issues map to repos via label prefix (`repo:xxx`).

**Tech Stack:** `lineark-sdk` 1.1 (typed Linear GraphQL SDK), existing `reqwest`/`chrono`/`serde` stack.

---

## Task 1: Add `lineark-sdk` workspace dependency

**Files:**
- Modify: `Cargo.toml` (workspace root, `[workspace.dependencies]` section)
- Modify: `crates/symphony/Cargo.toml` (`[dependencies]` section)

**Step 1: Add lineark-sdk to workspace deps**

In root `Cargo.toml`, add to `[workspace.dependencies]` (alphabetical, after `lazy_static`):
```toml
lineark-sdk = "1.1"
```

**Step 2: Add lineark-sdk to symphony crate deps**

In `crates/symphony/Cargo.toml`, add to `[dependencies]`:
```toml
lineark-sdk.workspace = true
```

**Step 3: Verify it compiles**

Run: `cargo check -p rara-symphony`
Expected: OK (no usage yet, just dependency added)

**Step 4: Commit**

```bash
git add Cargo.toml crates/symphony/Cargo.toml Cargo.lock
git commit -m "chore(symphony): add lineark-sdk dependency"
```

---

## Task 2: Add `identifier` field to `TrackedIssue` and fix all references

**Files:**
- Modify: `crates/symphony/src/event.rs`
- Modify: `crates/symphony/src/tracker.rs` (GitHubIssueTracker: set `identifier`)
- Modify: `crates/symphony/src/agent.rs` (tests: add `identifier` to `sample_issue()`)
- Modify: `crates/symphony/src/workflow.rs` (add `{{issue.identifier}}` template var, fix test helpers)
- Modify: `crates/symphony/src/orchestrator.rs` (fix test helpers)

**Step 1: Add `identifier` field to `TrackedIssue`**

In `event.rs`, add after `pub id: String,`:
```rust
    /// Human-readable identifier. GitHub: "42", Linear: "RAR-42".
    pub identifier: String,
```

**Step 2: Set `identifier` in `GitHubIssueTracker`**

In `tracker.rs`, in the `fetch_repo_issues` method, inside the `.map(|item| { ... })` closure, add `identifier` field to `TrackedIssue` construction:
```rust
                TrackedIssue {
                    id: format!("{}#{}", repo.name, item.number),
                    identifier: item.number.to_string(),
                    repo: repo.name.clone(),
```

**Step 3: Add `{{issue.identifier}}` to workflow template rendering**

In `workflow.rs` `render_prompt()`, add after the `{{issue.id}}` line:
```rust
    result = result.replace("{{issue.identifier}}", &ctx.issue.identifier);
```

**Step 4: Fix all test helpers that construct `TrackedIssue`**

Every test that builds a `TrackedIssue` needs `identifier` added. There are instances in:
- `agent.rs` `sample_issue()`: add `identifier: "42".to_owned(),` after `id`
- `orchestrator.rs` test `issue()` helper: add `identifier: number.to_string(),` after `id`
- `tracker.rs` test `issue()` helper: add `identifier: number.to_string(),` after `id`
- `workflow.rs` test issue construction: add `identifier: "42".to_owned(),` after `id`

**Step 5: Verify**

Run: `cargo check -p rara-symphony && cargo test -p rara-symphony`
Expected: All pass

**Step 6: Commit**

```bash
git add crates/symphony/src/
git commit -m "feat(symphony): add identifier field to TrackedIssue"
```

---

## Task 3: Add `TrackerConfig` and wire into `SymphonyConfig`

**Files:**
- Modify: `crates/symphony/src/config.rs`

**Step 1: Add `TrackerConfig` struct**

Add after the `default_*` functions, before `SymphonyConfig`:

```rust
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

        /// Label prefix for repo mapping (e.g. "repo:" → "repo:myorg/myrepo").
        #[serde(default = "default_repo_label_prefix")]
        repo_label_prefix: String,
    },
}
```

**Step 2: Add `tracker` field to `SymphonyConfig`**

Add to `SymphonyConfig` struct, after `enabled`:
```rust
    /// Issue tracker configuration.
    pub tracker: Option<TrackerConfig>,
```

**Step 3: Verify**

Run: `cargo check -p rara-symphony`
Expected: OK

**Step 4: Commit**

```bash
git add crates/symphony/src/config.rs
git commit -m "feat(symphony): add TrackerConfig enum (github/linear)"
```

---

## Task 4: Add `Linear` error variant

**Files:**
- Modify: `crates/symphony/src/error.rs`

**Step 1: Add variant**

Add to `SymphonyError` enum:
```rust
    #[snafu(display("linear API error: {message}"))]
    Linear { message: String },
```

**Step 2: Verify**

Run: `cargo check -p rara-symphony`
Expected: OK

**Step 3: Commit**

```bash
git add crates/symphony/src/error.rs
git commit -m "feat(symphony): add Linear error variant"
```

---

## Task 5: Implement `LinearIssueTracker`

**Files:**
- Modify: `crates/symphony/src/tracker.rs`

**Step 1: Add the `LinearIssueTracker` struct and constructor**

Add after `GitHubIssueTracker` impl block, before the helper functions:

```rust
/// Linear-backed issue tracker using the GraphQL API via `lineark-sdk`.
pub struct LinearIssueTracker {
    client: lineark_sdk::Client,
    project_slug: String,
    active_states: Vec<String>,
    terminal_states: Vec<String>,
    repo_label_prefix: String,
    repos: Vec<String>,
}

impl LinearIssueTracker {
    /// Create a new Linear issue tracker.
    ///
    /// # Arguments
    /// * `api_key` — Linear API key (resolved from config, not `$VAR` form)
    /// * `project_slug` — Linear project slugId
    /// * `active_states` — issue states that trigger dispatch
    /// * `terminal_states` — issue states considered terminal
    /// * `repo_label_prefix` — label prefix for repo mapping
    /// * `repos` — list of configured repo names for validation
    pub fn new(
        api_key: &str,
        project_slug: String,
        active_states: Vec<String>,
        terminal_states: Vec<String>,
        repo_label_prefix: String,
        repos: Vec<String>,
    ) -> Result<Self> {
        let client = lineark_sdk::Client::from_token(api_key).map_err(|e| {
            LinearSnafu {
                message: format!("failed to create Linear client: {e}"),
            }
            .build()
        })?;

        Ok(Self {
            client,
            project_slug,
            active_states,
            terminal_states,
            repo_label_prefix,
            repos,
        })
    }

    /// Extract the target repo from issue labels using the configured prefix.
    /// Returns `None` if no matching label is found.
    fn extract_repo(&self, labels: &[String]) -> Option<String> {
        for label in labels {
            if let Some(repo) = label.strip_prefix(&self.repo_label_prefix) {
                if self.repos.contains(&repo.to_owned()) {
                    return Some(repo.to_owned());
                }
            }
        }
        None
    }

    /// Parse the numeric portion from a Linear identifier like "RAR-42" → 42.
    fn parse_number(identifier: &str) -> u64 {
        identifier
            .rsplit('-')
            .next()
            .and_then(|n| n.parse().ok())
            .unwrap_or(0)
    }

    /// Map Linear priority (0=none, 1=urgent, 2=high, 3=medium, 4=low) to our u32 scale.
    fn map_priority(linear_priority: u32) -> u32 {
        match linear_priority {
            0 => u32::MAX, // no priority
            n => n,        // 1=urgent..4=low maps directly
        }
    }
}
```

**Step 2: Implement `IssueTracker` for `LinearIssueTracker`**

```rust
#[async_trait]
impl IssueTracker for LinearIssueTracker {
    async fn fetch_active_issues(&self) -> Result<Vec<TrackedIssue>> {
        let query = r#"
            query($projectSlug: String!, $states: [String!]!, $after: String) {
                issues(
                    filter: {
                        project: { slugId: { eq: $projectSlug } }
                        state: { name: { in: $states } }
                    }
                    first: 50
                    after: $after
                    orderBy: createdAt
                ) {
                    nodes {
                        id
                        identifier
                        title
                        description
                        priority
                        createdAt
                        state { name }
                        labels { nodes { name } }
                    }
                    pageInfo { hasNextPage endCursor }
                }
            }
        "#;

        let mut all_issues = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let variables = serde_json::json!({
                "projectSlug": self.project_slug,
                "states": self.active_states,
                "after": cursor,
            });

            let conn: lineark_sdk::pagination::Connection<serde_json::Value> = self
                .client
                .execute_connection(query, variables, "issues")
                .await
                .map_err(|e| {
                    LinearSnafu {
                        message: format!("failed to fetch issues: {e}"),
                    }
                    .build()
                })?;

            for node in &conn.nodes {
                let id = node["id"].as_str().unwrap_or_default().to_owned();
                let identifier = node["identifier"].as_str().unwrap_or_default().to_owned();
                let title = node["title"].as_str().unwrap_or_default().to_owned();
                let body = node["description"].as_str().map(String::from);
                let priority = node["priority"].as_u64().unwrap_or(0) as u32;
                let created_at_str = node["createdAt"].as_str().unwrap_or_default();
                let created_at = created_at_str
                    .parse::<DateTime<Utc>>()
                    .unwrap_or_else(|_| Utc::now());

                let labels: Vec<String> = node["labels"]["nodes"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|l| l["name"].as_str().map(|s| s.to_lowercase()))
                            .collect()
                    })
                    .unwrap_or_default();

                let repo = match self.extract_repo(&labels) {
                    Some(r) => r,
                    None => {
                        tracing::warn!(
                            identifier = %identifier,
                            "no matching repo label (prefix '{}'), skipping",
                            self.repo_label_prefix,
                        );
                        continue;
                    }
                };

                let number = Self::parse_number(&identifier);

                all_issues.push(TrackedIssue {
                    id,
                    identifier,
                    repo,
                    number,
                    title,
                    body,
                    labels,
                    priority: Self::map_priority(priority),
                    state: IssueState::Active,
                    created_at,
                });
            }

            if conn.page_info.has_next_page {
                cursor = conn.page_info.end_cursor;
            } else {
                break;
            }
        }

        sort_issues(&mut all_issues);
        Ok(all_issues)
    }

    async fn fetch_issue_state(&self, _repo: &str, number: u64) -> Result<IssueState> {
        // For Linear, `number` is unused in this path. The `_repo` param
        // is actually the GraphQL ID when called from retry logic.
        // We query by the issue ID stored in TrackedIssue.id.
        let query = r#"
            query($id: String!) {
                issue(id: $id) {
                    state { name }
                }
            }
        "#;

        let variables = serde_json::json!({ "id": _repo });

        let result: serde_json::Value = self
            .client
            .execute(query, variables, "issue")
            .await
            .map_err(|e| {
                LinearSnafu {
                    message: format!("failed to fetch issue state: {e}"),
                }
                .build()
            })?;

        let state_name = result["state"]["name"]
            .as_str()
            .unwrap_or_default();

        if self.terminal_states.iter().any(|s| s.eq_ignore_ascii_case(state_name)) {
            Ok(IssueState::Terminal)
        } else {
            Ok(IssueState::Active)
        }
    }
}
```

**Step 3: Add `LinearSnafu` import**

At the top of `tracker.rs`, add to the error import:
```rust
use crate::error::{GitHubSnafu, LinearSnafu, Result};
```

**Step 4: Verify**

Run: `cargo check -p rara-symphony`
Expected: OK

**Step 5: Add tests for helper methods**

Add to the `#[cfg(test)] mod tests` block:

```rust
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
        assert_eq!(LinearIssueTracker::map_priority(1), 1); // urgent
        assert_eq!(LinearIssueTracker::map_priority(2), 2); // high
        assert_eq!(LinearIssueTracker::map_priority(3), 3); // medium
        assert_eq!(LinearIssueTracker::map_priority(4), 4); // low
    }
```

**Step 6: Run tests**

Run: `cargo test -p rara-symphony`
Expected: All pass

**Step 7: Commit**

```bash
git add crates/symphony/src/tracker.rs
git commit -m "feat(symphony): implement LinearIssueTracker"
```

---

## Task 6: Fix orchestrator retry parsing

**Files:**
- Modify: `crates/symphony/src/orchestrator.rs`

**Step 1: Store issue metadata for retry instead of parsing from ID**

The current `handle_retry` method parses `repo` and `number` from the issue ID string using `#` splitting. This won't work for Linear UUIDs.

Replace the retry logic: instead of parsing from `issue_id`, store `TrackedIssue` in the retry map so we have `repo` and `number` available.

Change `RetryEntry` to:
```rust
struct RetryEntry {
    attempt: u32,
    issue: TrackedIssue,
}
```

Update `handle_agent_failed` to store the issue from `RunState`:
```rust
    fn handle_agent_failed(&mut self, issue_id: &str, reason: &str) {
        warn!(issue_id = %issue_id, reason = %reason, "agent failed");

        let run_state = self.running.remove(issue_id);

        let entry = self
            .retries
            .entry(issue_id.to_owned())
            .and_modify(|e| e.attempt += 1)
            .or_insert_with(|| RetryEntry {
                attempt: 1,
                issue: run_state
                    .as_ref()
                    .expect("run_state must exist for failed agent")
                    .issue
                    .clone(),
            });
        let attempt = entry.attempt;
        // ... rest unchanged
    }
```

Update `handle_retry` to use stored issue data instead of parsing:
```rust
    async fn handle_retry(&mut self, issue_id: &str) -> Result<()> {
        info!(issue_id = %issue_id, "processing retry");

        self.claimed.remove(issue_id);
        self.running.remove(issue_id);

        let entry = match self.retries.get(issue_id) {
            Some(e) => e,
            None => {
                warn!(issue_id = %issue_id, "no retry entry found, dropping");
                return Ok(());
            }
        };
        let repo = &entry.issue.repo;
        let number = entry.issue.number;

        match self.tracker.fetch_issue_state(repo, number).await {
            Ok(IssueState::Active) => {
                info!(issue_id = %issue_id, "issue still active, triggering re-poll");
                if let Err(e) = self.handle_poll_tick().await {
                    error!(error = %e, "re-poll after retry failed");
                }
            }
            Ok(IssueState::Terminal) => {
                info!(issue_id = %issue_id, "issue is now terminal, dropping retry");
                self.retries.remove(issue_id);
            }
            Err(e) => {
                warn!(issue_id = %issue_id, error = %e, "failed to fetch issue state for retry");
            }
        }

        Ok(())
    }
```

**Step 2: Verify**

Run: `cargo check -p rara-symphony && cargo test -p rara-symphony`
Expected: All pass

**Step 3: Commit**

```bash
git add crates/symphony/src/orchestrator.rs
git commit -m "refactor(symphony): store issue in RetryEntry instead of parsing ID"
```

---

## Task 7: Wire tracker construction in `SymphonyService`

**Files:**
- Modify: `crates/symphony/src/service.rs`

**Step 1: Update `SymphonyService` to construct tracker based on config**

Replace the hardcoded `GitHubIssueTracker` in `run()`:

```rust
use crate::config::TrackerConfig;
use crate::tracker::{GitHubIssueTracker, LinearIssueTracker};

// In run():
        let tracker: Box<dyn crate::tracker::IssueTracker> = match &self.config.tracker {
            Some(TrackerConfig::Linear {
                api_key,
                project_slug,
                active_states,
                terminal_states,
                repo_label_prefix,
                ..
            }) => {
                let resolved_key = resolve_env_var(api_key);
                let repo_names = self.config.repos.iter().map(|r| r.name.clone()).collect();
                Box::new(LinearIssueTracker::new(
                    &resolved_key,
                    project_slug.clone(),
                    active_states.clone(),
                    terminal_states.clone(),
                    repo_label_prefix.clone(),
                    repo_names,
                )?)
            }
            Some(TrackerConfig::Github { api_key }) => {
                let token = api_key.as_ref().map(|k| resolve_env_var(k));
                Box::new(GitHubIssueTracker::new(
                    self.config.repos.clone(),
                    token,
                ))
            }
            None => {
                // Backward compat: default to GitHub with optional token from constructor
                Box::new(GitHubIssueTracker::new(
                    self.config.repos.clone(),
                    self.github_token.clone(),
                ))
            }
        };
```

Add helper function at bottom of `service.rs`:
```rust
/// Resolve a `$ENV_VAR` reference to its value, or return the string as-is.
fn resolve_env_var(value: &str) -> String {
    if let Some(var_name) = value.strip_prefix('$') {
        std::env::var(var_name).unwrap_or_default()
    } else {
        value.to_owned()
    }
}
```

**Step 2: Remove unused `github_token` import if no longer needed**

The `github_token` field stays for backward compat (when `tracker` is `None`).

**Step 3: Verify**

Run: `cargo check -p rara-symphony`
Expected: OK

**Step 4: Commit**

```bash
git add crates/symphony/src/service.rs
git commit -m "feat(symphony): wire LinearIssueTracker in SymphonyService"
```

---

## Task 8: Add `{{issue.identifier}}` to agent prompt and update default prompt

**Files:**
- Modify: `crates/symphony/src/agent.rs`

**Step 1: Update default prompt to use identifier**

In `default_prompt()`, change the format string to use `identifier` for display and `number` for commit refs:
```rust
    fn default_prompt(&self, task: &AgentTask) -> String {
        let id_display = &task.issue.identifier;
        // ... use id_display in the prompt header
```

This is optional — the current `#{number}` still works since `number` is extracted from identifier. No change needed if we're OK with `#42` instead of `#RAR-42`.

**Step 2: Verify all tests pass**

Run: `cargo test -p rara-symphony`
Expected: All pass

**Step 3: Commit**

```bash
git add crates/symphony/src/
git commit -m "feat(symphony): Linear issue tracker integration complete"
```
