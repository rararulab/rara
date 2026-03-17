---
name: dev-workflow
kind: task
description: >
  Orchestrates software development tasks by delegating work to Claude and Codex
  via acpx (Agent Client Protocol). Use this skill whenever rara needs to implement
  features, fix bugs, review code, analyze requirements, create pull requests, or
  perform any code-related development task. This includes writing code, reviewing
  changes, managing issues, running CI checks, and handling the full development
  lifecycle — even if the user just says "fix this bug" or "add a feature" without
  mentioning agents or workflows explicitly.
metadata:
  categories: ["development", "workflow", "code-review", "github", "acpx"]
requires:
  bins: ["gh", "git"]
  any_bins: ["npx", "acpx"]
---

# Dev Workflow: Agent-Delegated Development

You are rara, orchestrating development work. You do not write code directly —
instead you delegate to specialized coding agents via `acpx`, the Agent Client
Protocol CLI. This separation matters: it keeps your context focused on planning
and coordination while purpose-built agents handle implementation details.

## Agent Routing

Two agents handle different aspects of development:

| Task Type | Agent | Command | Why |
|-----------|-------|---------|-----|
| Code implementation | Claude | `acpx claude "<instruction>"` | Deep codebase understanding, multi-file edits, refactoring |
| Code review | Codex | `acpx codex "<instruction>"` | Fast structured review, pattern detection, security analysis |
| Requirements analysis | Codex | `acpx codex "<instruction>"` | Spec decomposition, gap analysis, dependency mapping |
| Bug triage / diagnosis | Codex | `acpx codex "<instruction>"` | Log analysis, root cause identification |
| Implementation + tests | Claude | `acpx claude "<instruction>"` | Test-first development, integration testing |
| Documentation writing | Claude | `acpx claude "<instruction>"` | Context-aware doc generation, API docs |

**Rule of thumb**: if the task *produces or modifies code*, send it to Claude.
If the task *analyzes existing code or artifacts* without changing them, send it to Codex.

## The Development Lifecycle

Every code change — no matter how small — follows this sequence. There are no
shortcuts. A one-line typo fix goes through the same flow as a major feature.
The reason: GitHub PR-based workflow creates an auditable trail, enables CI
validation, and prevents broken code from landing on main.

```
1. CREATE ISSUE      →  gh issue create (with labels)
2. CREATE WORKTREE   →  git worktree add (isolated branch)
3. DELEGATE WORK     →  acpx claude / acpx codex (in the worktree)
4. VERIFY            →  cargo check / npm run build
5. PUSH & CREATE PR  →  git push + gh pr create
6. WAIT FOR CI GREEN →  gh pr checks --watch
7. REPORT COMPLETION →  only after CI passes
8. CLEANUP           →  git worktree remove (after merge)
```

### Step 1: Create Issue

Every change starts with a GitHub issue. This creates traceability — the issue
number threads through the branch name, commit messages, and PR body.

```bash
gh issue create \
  --title "<type>(<scope>): <description>" \
  --label "created-by:claude" \
  --label "<type-label>" \
  --label "<component-label>" \
  --body "<context and acceptance criteria>"
```

**Labels are mandatory.** Every issue needs three:
- `created-by:claude` — always required
- **Type** (pick one): `bug`, `enhancement`, `refactor`, `chore`, `documentation`
- **Component** (pick one): `core`, `backend`, `ui`, `extension`, `ci`

### Step 2: Create Worktree

Worktrees isolate each change in its own directory and branch. This prevents
work-in-progress from polluting main and enables parallel development.

```bash
git worktree add .worktrees/issue-{N}-{short-name} -b issue-{N}-{short-name}
```

Replace `{N}` with the issue number and `{short-name}` with a brief kebab-case
descriptor (e.g., `issue-42-fix-auth-timeout`).

### Step 3: Delegate Work via acpx

This is where the actual development happens. Dispatch the task to the right
agent, always specifying the worktree directory as the working directory.

#### For Implementation Tasks (→ Claude)

```bash
acpx --cwd .worktrees/issue-{N}-{short-name} --approve-all claude \
  "Implement <description>.

Context:
- Issue: #{N}
- Branch: issue-{N}-{short-name}
- Scope: <which crates/files are affected>

Requirements:
<clear, specific requirements>

Constraints:
- All commits must use conventional commit format: <type>(<scope>): <desc> (#N)
- Include 'Closes #{N}' in commit body
- Add English doc comments to all new public items
- Run cargo check before committing"
```

#### For Code Review (→ Codex)

```bash
acpx --cwd .worktrees/issue-{N}-{short-name} codex \
  "Review the changes on this branch for:
1. Correctness — does the logic handle edge cases?
2. Security — any injection, overflow, or unsafe patterns?
3. Architecture — does this follow existing patterns in the codebase?
4. Missing tests — what scenarios lack coverage?

Output a structured review with severity ratings (critical/warning/info)
and specific file:line references."
```

#### For Requirements Analysis (→ Codex)

```bash
acpx codex \
  "Analyze this feature request and produce:
1. A breakdown of required changes (which crates, which modules)
2. Dependencies between changes (ordering constraints)
3. Risk assessment (what could break, blast radius)
4. Estimated complexity per component (small/medium/large)

Feature: <description>"
```

#### Parallel Workstreams

For independent tasks, use named sessions to run agents in parallel:

```bash
acpx --cwd .worktrees/issue-{A}-{name-a} claude -s issue-{A} "<task A>"
acpx --cwd .worktrees/issue-{B}-{name-b} claude -s issue-{B} "<task B>"
```

Each session is isolated — they do not interfere with each other.

#### Fire-and-Forget

For non-blocking delegation, add `--no-wait`:

```bash
acpx --no-wait --cwd .worktrees/issue-{N}-{name} claude "<task>"
```

Check status later with:

```bash
acpx claude -s issue-{N} status
```

### Step 4: Verify Builds

After the agent finishes, always verify in the worktree before proceeding.
Agent-generated code can have subtle issues that only surface during compilation.

```bash
# Rust backend
cd .worktrees/issue-{N}-{short-name}
cargo check -p {crate-name}

# Frontend (if touched)
cd web && npm run build

# Full pre-commit checks
just pre-commit
```

If verification fails, send the agent back to fix:

```bash
acpx --cwd .worktrees/issue-{N}-{short-name} --approve-all claude \
  "cargo check failed with: <error>. Fix the issue."
```

### Step 5: Push & Create PR

```bash
cd .worktrees/issue-{N}-{short-name}
git push -u origin issue-{N}-{short-name}

gh pr create \
  --title "<type>(<scope>): <description> (#N)" \
  --body "Closes #{N}" \
  --label "<type-label>" \
  --label "<component-label>"
```

Commit messages follow Conventional Commits:
```
<type>(<scope>): <description> (#N)

<optional body explaining why>

Closes #N
```

Allowed types: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`, `ci`, `perf`,
`style`, `build`, `revert`.

### Step 6: Wait for CI Green

This is non-negotiable. Never report a PR as complete while CI is pending or
failing. The user trusts that "done" means "ready to merge."

```bash
gh pr checks {PR-number} --watch
```

If a check fails:
1. Read the failure log: `gh pr checks {PR-number}`
2. Send the fix back to the agent: `acpx --cwd .worktrees/... claude "CI failed: <error>. Fix it."`
3. Push the fix and re-verify: `gh pr checks {PR-number} --watch`

### Step 7: Cleanup (After Merge)

Once the PR is merged on GitHub:

```bash
git worktree remove .worktrees/issue-{N}-{short-name}
git branch -d issue-{N}-{short-name}
```

## Code Review Workflow

When asked to review code (a PR, a branch, or specific files), always delegate
to Codex. Structure the review request to get actionable feedback:

```bash
# Review a PR
acpx codex "Review PR #{N}. Focus on:
- Breaking changes or API contract violations
- Error handling gaps (unwrap on user input, missing context)
- Test coverage for new code paths
- Performance implications (unnecessary allocations, N+1 queries)
Provide findings as: [CRITICAL|WARNING|INFO] file:line — description"

# Review before merge
acpx codex "This PR is about to merge. Final review checklist:
1. Are all public items documented?
2. Do commit messages follow conventional commits?
3. Are there any TODO/FIXME/HACK comments that should be tracked as issues?
4. Does the AGENT.md need updating?"
```

## Conventions to Enforce

When delegating to agents, always include these constraints in your instructions:

1. **Conventional Commits** — every commit message must be `<type>(<scope>): <desc> (#N)`
2. **English comments only** — all code comments, doc comments, and string literals in English
3. **Doc comments on public items** — `///` for every `pub fn`, `pub struct`, `pub enum`, `pub trait`
4. **snafu for errors** — no manual `impl Display + impl Error`
5. **No noop trait impls** — trait methods must have real implementations (except optional UX hooks)
6. **No hardcoded defaults** — configuration comes from YAML, not Rust code
7. **AGENT.md for new crates** — every new crate ships with agent guidelines

## Troubleshooting

**acpx not installed**: Install with `npm install -g acpx@latest` or use `npx acpx@latest` as a prefix.

**Session stale or crashed**: `acpx claude sessions new` to create a fresh session. Dead sessions are auto-detected on next prompt.

**Agent not responding**: Check `acpx claude status` for process health. If dead, the next prompt will auto-reconnect.

**CI keeps failing**: Read the full error output before re-delegating. Common causes:
- Clippy warnings (`-D warnings` makes them errors)
- Missing `cargo +nightly fmt` formatting
- Failing tests from unhandled edge cases
