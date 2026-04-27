# Development Workflow — Issue → Worktree → PR → Merge

**Every code change — no matter how small — MUST follow this workflow.** There are zero exceptions: single-line fixes, typo corrections, config tweaks, doc updates, and refactors all go through issue + worktree + PR. The main agent must NEVER directly edit source files on the `main` branch.

```
1. CREATE ISSUE    →  gh issue create + labels
2. CREATE WORKTREE →  git worktree add .worktrees/issue-{N}-{name} -b issue-{N}-{name}
3. WORK            →  All edits happen inside the worktree
4. VERIFY          →  cargo check + npm run build on worktree
5. PUSH & PR       →  git push -u origin + gh pr create
6. WAIT FOR CI     →  gh pr checks {N} --watch (must be green)
7. CODE REVIEW     →  /code-review-expert skill, fix findings, loop until APPROVE
8. MERGE           →  gh pr merge {N} --squash --delete-branch
9. CLEANUP         →  git worktree remove + git branch -d
```

## Step 1: Create Issue

Issues MUST use the GitHub issue templates defined in `.github/ISSUE_TEMPLATE/`. Pick the template matching the change type:

| Template | Use when |
|----------|----------|
| `feature_request.yml` | New feature or enhancement |
| `bug_report.yml` | Bug fix |
| `refactor.yml` | Code refactor or technical improvement |
| `chore.yml` | CI, dependencies, tooling, maintenance |

```bash
# Example: feature request
gh issue create --template feature_request.yml \
  --title "feat(kernel): event queue sharding" \
  --body "$(cat <<'EOF'
### Description
Event queue sharding to improve throughput.

### Component
kernel (core runtime, heartbeat, event bus)

### Alternatives considered
None.
EOF
)" --label "agent:claude" --label "core"

# Example: bug report
gh issue create --template bug_report.yml \
  --title "fix(web): session token not refreshed" \
  --body "$(cat <<'EOF'
### Description
Session token expires but is not refreshed automatically.

### Component
web (frontend, UI)

### Steps to reproduce
1. Login and wait 30 minutes
2. Attempt any action
3. 401 error

### Logs / Error output
401 Unauthorized

### Version
rara 0.0.1
EOF
)" --label "agent:claude" --label "ui"
```

**Issue Labels** (all issues MUST have proper labels):
- **Agent** (required for agent-created issues): `agent:claude`, `agent:codex` — use the label matching the agent performing the operation
- **Type**: auto-applied by the template (`enhancement`, `bug`, `refactor`, `chore`)
- **Component** (pick one): `core`, `backend`, `ui`, `extension`, `ci`

## Step 2: Create Worktree
```bash
git worktree add .worktrees/issue-{N}-{short-name} -b issue-{N}-{short-name}
```

## Step 3: Work in Worktree
- **All code edits happen exclusively inside the worktree directory** — never in the main checkout
- The main agent may dispatch a subagent to the worktree, or work there directly
- When dispatching, prefer `subagent_type: implementer` (defined in `.claude/agents/implementer.md`) over `general-purpose` — it carries the project's commit/verify/PR conventions and the config-schema guardrail learned from #1907
- Independent issues can be dispatched **in parallel** (each in its own worktree)
- All work should be committed before moving to the next step

## Step 4: Verify Builds
After subagent completes, verify in the worktree:
```bash
cargo check -p {crate-name}   # Rust backend
cd web && npm run build        # Frontend (if touched)
```

## Pre-commit Checks (prek)

The project uses [prek](https://github.com/j178/prek) for pre-commit hooks. The **final commit** in any PR must pass all checks — intermediate commits during development don't need to pass.

Setup (required once after clone):
```bash
brew install prek              # Install prek
prek install                   # Install git hooks into .git/hooks
```

Hooks configured in `.pre-commit-config.yaml`:
- `cargo check --all --all-targets`
- `cargo +nightly fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features --no-deps -- -D warnings`
- `RUSTDOCFLAGS="-D warnings" cargo +nightly doc --workspace --no-deps --document-private-items`

Triggers on: `.rs`, `.toml`, `Cargo.lock`, `rust-toolchain.toml` changes.

Run all checks manually:
```bash
prek run --all-files           # Run all hooks
just pre-commit                # Alternative: fmt + clippy + check + test
```

If pre-commit hook blocks a commit during development, fix issues before the final commit. Do NOT use `--no-verify` to skip hooks.

## Step 5: Push & Create PR

PRs use the template at `.github/pull_request_template.md`. Fill in all sections.

```bash
git push -u origin issue-{N}-{short-name}
gh pr create --title "fix(scope): description (#N)" --body "$(cat <<'EOF'
## Summary

Brief description of the changes.

## Type of change

| Type | Label |
|------|-------|
| Bug fix | `bug` |

## Component

`core`

## Closes

Closes #N

## Test plan

- [x] `just test` passes
- [x] `just lint` passes
- [x] Tested locally
EOF
)" --label "bug" --label "core"
```
- Commit message must include `Closes #N` so the issue is auto-closed when PR merges
- Never merge locally — all merges happen through GitHub PR
- **PR Labels** (all PRs MUST have proper labels):
  - **Type** (pick one): `bug`, `enhancement`, `refactor`, `chore`, `documentation`
  - **Component** (pick one): `core`, `backend`, `ui`, `extension`, `ci`
  - Note: a `labeler.yml` workflow auto-labels PRs by file path, but agents must still add type + component labels explicitly via `--label` flags

## Step 6: Wait for CI Green (MANDATORY)

After creating the PR, **you MUST verify that all CI checks pass before moving on.**

```bash
gh pr checks {PR-number} --watch    # Wait for all checks to complete
```

- If any check fails, investigate and fix in the worktree, push again, and re-verify
- Do NOT proceed to review or merge while CI is still pending or failing

## Step 7: Code Review (MANDATORY)

After CI is green, run a structured code review with the **`/code-review-expert`** skill — the main agent invokes the skill via the `Skill` tool, or dispatches the **`reviewer`** subagent (`.claude/agents/reviewer.md`) which wraps the same skill and adds the cross-PR regression-decision check (the #1907 lesson: catch silent reversals of recent design decisions). The skill produces a verdict (APPROVE / REQUEST_CHANGES / COMMENT) plus findings graded P0–P3.

The agent never approves its own diff in lieu of running the skill — it comes in cold and catches what the implementer missed.

- **REQUEST_CHANGES**: fix every blocking finding (P0/P1) in the worktree, push, re-run the skill. Loop until APPROVE.
- **APPROVE with P2/P3 nits**: address only the nits that are clearly worth fixing in this PR. Don't stall on stylistic preferences.
- **APPROVE clean**: proceed to merge.

This is non-negotiable — even one-line fixes go through it. The skill is fast; the cost of skipping it (regressions like #1810) is high.

## Step 8: Merge to Main

Once CI is green AND the review is APPROVE (with all blocking findings handled), merge without further confirmation — green CI + clean review IS the merge signal.

```bash
gh pr merge {N} --squash --delete-branch
```

Use `--squash` so the merged commit on `main` matches the Conventional Commit subject. `--delete-branch` removes the remote branch; the local branch + worktree are removed in Step 9.

## Step 9: Cleanup
```bash
git worktree remove .worktrees/issue-{N}-{short-name}
git branch -D issue-{N}-{short-name}    # -D because the branch is gone on origin
```

## Parallel Execution

When user requests involve multiple independent changes, split into separate issues and dispatch subagents in parallel:
- Each subagent gets its own worktree, branch, and PR
- PRs are reviewed and merged independently on GitHub
