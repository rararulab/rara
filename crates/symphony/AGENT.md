# rara-symphony — Agent Guidelines

## Purpose

Issue tracker to coding agent synchronization bridge — polls issue trackers (GitHub Issues, Linear) for ready tasks, provisions git worktrees, spawns `ralph` agent processes, and advances issue state on completion.

## Architecture

### Key modules

- `src/config.rs` — `SymphonyConfig`, `TrackerConfig` (GitHub/Linear variants), `AgentConfig`, `RepoConfig`. All config is YAML-driven via `rara-app`.
- `src/service.rs` — `SymphonyService::run()` main poll loop. `IssueRuntime` manages running/failed issue state, worktree lifecycle, and child process reaping.
- `src/tracker.rs` — `IssueTracker` trait with `GitHubIssueTracker` and `LinearIssueTracker` implementations. Fetches active issues, transitions state.
- `src/agent.rs` — `RalphAgent` wraps `ralph init` + `ralph run` subprocess spawning. `AgentTask` carries issue context + optional workflow content.
- `src/workspace.rs` — `WorkspaceManager` creates/cleans git worktrees via `git2`. `WorkspaceInfo` tracks path, branch, and whether newly created.
- `src/error.rs` — `snafu`-based error types.

### Data flow

1. `SymphonyService::run()` polls `IssueTracker::fetch_active_issues()` on `poll_interval`.
2. For each new ready issue, `IssueRuntime::start_issue()` provisions a worktree via `WorkspaceManager`.
3. Reads optional `WORKFLOW.md` from the worktree root for task context.
4. Spawns `ralph init` then `ralph run` as a child process in the worktree directory.
5. stdout/stderr are captured to per-issue log files under `~/.config/rara/ralpha/logs/<repo>/`.
6. On child exit, transitions issue state (success → `ToVerify`, failure → stays in `In Progress`).
7. If review is enabled, spawns a reviewer on the same workspace (review phase).
8. If verify is enabled, issues in `completed_issue_state` (e.g. "ToVerify") are picked up by the verify pipeline on a subsequent poll cycle. The verifier reuses the existing workspace or recovers it from the remote branch ref.
9. On verify success, transitions to `verify.completed_state` (e.g. "WaitingApprove") and cleans up. On verify failure, adds `AutoVerifyFailed` label and moves to failed for retry.
10. Cleans up worktree on successful completion or when issue reaches terminal state externally.

### Public API

- `SymphonyConfig` — top-level config (re-exported).
- `SymphonyService::new(config, shutdown, github_token)` + `run()`.

## Critical Invariants

- `max_concurrent_agents` is enforced globally and per-repo for coding agents — never bypass the slot check.
- `verify.max_concurrent` is enforced separately for verify agents.
- Worktree cleanup must happen after child process termination — killing a child without cleanup leaks git worktrees.
- `TrackerConfig` API keys support `$ENV_VAR` syntax — resolved at runtime via `resolve_env_var()`.
- Log files are truncated on each new run for the same issue — not appended.

## What NOT To Do

- Do NOT run `ralph` commands outside of a provisioned worktree — the agent expects a git worktree context.
- Do NOT transition issue state without checking the child exit status — success/failure determines the target state.
- Do NOT store mutable runtime state in `SymphonyConfig` — config is immutable after construction.

## Dependencies

**Upstream:** `rara-paths` (log/workspace directories), `git2` (worktree management), `lineark-sdk` (Linear API), `reqwest` (GitHub API).

**Downstream:** `rara-app` (starts the service, provides config).

**External services:** GitHub Issues API, Linear GraphQL API, `ralph` CLI binary.
