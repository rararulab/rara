---
name: dev
description: "Autonomous development pipeline: requirement → design → implement → review → ship PR."
---

# /dev — Autonomous Development Pipeline

One command, full cycle: requirement → design → implement → review → ship.

**Iron Law:** Every analysis step, decision, and review finding MUST be recorded on GitHub (issue comments + PR comments). The pipeline's work process must be fully traceable — never let conclusions exist only in conversation context.

**User intervenes only twice:**
1. After Phase 1: confirm the plan
2. After Phase 4: see the final result

**Announce at start:** "Running /dev pipeline for: {requirement}"

---

## Phase 0: ISSUE CREATION

Create the tracking issue **before any analysis begins**. This issue is the work log for the entire pipeline.

```bash
gh issue create --title "{type}({scope}): {description}" \
  --body "$(cat <<'EOF'
### Description
{requirement description and initial context}

### Component
{crate or area} ({description})

### Alternatives considered
TBD — will be analyzed in design phase.
EOF
)" --label "agent:claude" --label "{type-label}" --label "{component-label}"
```

Note: `--template` flag cannot be used with `--body`. The body MUST follow the template's field structure (### Description, ### Component, ### Alternatives considered).

Save the issue number as `{ISSUE}` — all subsequent comments reference it.

---

## Phase 1: DESIGN

### Step 1.1: Context Gathering

Gather project context silently (no output to user):

1. Read the project's `CLAUDE.md` and relevant `AGENT.md` files for the affected area
2. Use the Glob tool to search `docs/plans/*.md` for existing related design documents
3. Search GitHub issues for related discussions:
   ```bash
   gh issue list --search "{keywords}" --limit 5
   ```
4. Read relevant source code in the affected crates/modules

**Post findings to issue:**
```bash
gh issue comment {ISSUE} --body "$(cat <<'EOF'
## Context Investigation

**Related code:**
- `{file}`: {what it does and how it relates}
- ...

**Related issues/PRs:** {list or "none found"}
**Existing design docs:** {list or "none found"}
**Key observations:** {what the codebase already has, gaps identified}
EOF
)"
```

### Step 1.2: Brainstorm & Design Doc

Based on the gathered context:

1. Analyze the requirement — identify the core problem being solved
2. Propose 2-3 implementation approaches with trade-offs
3. **Autonomously select** the recommended approach — do NOT ask the user
4. Consider: architectural fit, complexity, existing patterns in the codebase, CLAUDE.md constraints

The design doc is drafted in memory during Phase 1 and physically written to `docs/plans/YYYY-MM-DD-{topic}-design.md` inside the worktree created in Phase 2.

The design doc MUST include:
- **Goal:** one sentence
- **Approach:** selected approach with reasoning
- **Affected crates/modules:** list with brief rationale
- **Key decisions:** any non-obvious choices made
- **Edge cases:** identified risks and how they're handled
- **Implementation steps:** numbered list of discrete tasks

**Post analysis to issue:**
```bash
gh issue comment {ISSUE} --body "$(cat <<'EOF'
## Design Analysis

**Approaches considered:**
1. {approach 1} — {trade-off}
2. {approach 2} — {trade-off}
3. {approach 3} — {trade-off}

**Selected:** Approach {N}
**Reasoning:** {why this approach wins}

**Key decisions:**
- {decision 1}: {rationale}
- {decision 2}: {rationale}

**Implementation steps:**
1. {step}
2. {step}
...
EOF
)"
```

### Step 1.3: Plan Review (autonomous loop)

Dispatch a **code-reviewer subagent** (via the Agent tool with `subagent_type: "superpowers:code-reviewer"`) to review the design doc. Provide the design doc content and the review dimensions in the prompt.

Review dimensions:
- Architectural soundness — does it fit the existing codebase?
- Compatibility — will it break existing functionality?
- Edge cases — are failure modes handled?
- Performance — any obvious bottlenecks?
- CLAUDE.md compliance — snafu errors, bon builders, functional style, etc.

**If issues found:**
1. Analyze each issue — read relevant code, search for conventions
2. If needed, web search for best practices
3. Revise the design doc
4. Re-review (max 2 rounds total)

**Post review result to issue:**
```bash
gh issue comment {ISSUE} --body "$(cat <<'EOF'
## Plan Review

**Round {N} result:** {clean / issues found}
{if issues: list each issue and how it was resolved}

**Final plan status:** Approved — proceeding to user confirmation.
EOF
)"
```

**If clean:** proceed to Step 1.4

### Step 1.4: Present to User

Output a concise plan summary to the user:

```
## /dev Plan Summary

**Issue:** #{ISSUE}
**Goal:** {one sentence}
**Approach:** {2-3 sentences}
**Key decisions:**
- {decision 1}
- {decision 2}

**Affected crates:** {list}
**Estimated steps:** {N tasks}

Design doc: docs/plans/YYYY-MM-DD-{topic}-design.md

Reply "ok" to proceed, or provide feedback.
```

**Wait for user confirmation.** If the user provides feedback:
- Revise the plan accordingly
- Re-run Step 1.3 review
- Present again

---

## Phase 2: IMPLEMENT

### Step 2.1: Scale Judgment

Parse the plan and count independent sub-tasks:

- **Small task** (< 3 independent steps): single worktree path (Step 2.2a)
- **Large task** (3+ independent steps that can run in parallel): multi-worktree path (Step 2.2b)

Independence criteria: tasks that don't modify the same files and don't depend on each other's output.

**Post to issue:**
```bash
gh issue comment {ISSUE} --body "$(cat <<'EOF'
## Implementation Start

**Scale:** {small / large} task
**Path:** {single worktree / stacked PRs with N sub-tasks}
**Branch:** `issue-{ISSUE}-{name}`
EOF
)"
```

### Step 2.2a: Small Task — Single Worktree

```bash
git worktree add .worktrees/issue-{ISSUE}-{name} -b issue-{ISSUE}-{name}
```

Dispatch a **subagent** (via the Agent tool) to the worktree with the full plan.

The subagent prompt MUST include:
- The full implementation plan from the design doc
- The worktree path to work in
- The issue number `{ISSUE}` and instruction to post progress comments:
  ```bash
  gh issue comment {ISSUE} --body "Progress: {what was just completed}"
  ```
- Instruction to follow CLAUDE.md conventions
- Instruction to run `cargo check -p {crate}` after each significant change
- Instruction to commit after each logical step with conventional commit messages

### Step 2.2b: Large Task — Multi-Worktree Parallel (Stacked PRs)

```bash
# Create feature branch from origin/main (no checkout needed)
git fetch origin main
git branch feat/{name} origin/main
git push -u origin feat/{name}
```

For each independent sub-task:
1. Create a sub-issue referencing `{ISSUE}`
2. Create a worktree branching from `feat/{name}`
3. Dispatch a subagent (via the Agent tool, with `run_in_background: true` for parallel execution)
   - Each subagent posts progress comments on its own sub-issue

After all subagents complete:
- Verify each worktree's changes compile
- Each sub-branch creates a PR targeting `feat/{name}` (never merge locally — use GitHub PR per stacked-prs.md)
- Merge sub-PRs in order via GitHub

**Partial failure handling:** If some subagents succeed and others fail:
- Keep successful worktrees and their commits
- Report which sub-tasks failed and why
- Escalate to user with options: retry failed tasks, proceed with partial implementation, or abort

### Step 2.3: Build Verification

After implementation completes, verify in the worktree:

```bash
cargo check -p {crate}
cargo clippy -p {crate} --all-targets --all-features --no-deps -- -D warnings
cargo test -p {crate}
```

If frontend was touched:
```bash
cd web && npm run build
```

**Post verification result to issue:**
```bash
gh issue comment {ISSUE} --body "$(cat <<'EOF'
## Build Verification

- cargo check: {pass/fail}
- cargo clippy: {pass/fail}
- cargo test: {pass/fail}
{- npm run build: {pass/fail}  # if frontend touched}

**Status:** {Ready for review / Fixing issues...}
EOF
)"
```

**If verification fails:**
- The implementing subagent self-fixes and retries (max 3 times)
- After 3 failures: escalate to user with error details

---

## Phase 3: REVIEW & FIX

### Step 3.0: Create Draft PR

Create a draft PR **before** starting review, so review findings can be posted as PR comments.

```bash
git push -u origin {branch}
gh pr create --draft --title "{type}({scope}): {description} (#{ISSUE})" --body "$(cat <<'EOF'
## Summary

{what was done, 2-3 sentences}

## Type of change

| Type | Label |
|------|-------|
| {type} | `{label}` |

## Component

`{component}`

## Closes

Closes #{ISSUE}

## Test plan

- [ ] `cargo check` passes
- [ ] `cargo clippy` passes
- [ ] `cargo test` passes
- [ ] Code review clean

## Review Log

_Review in progress..._
EOF
)" --label "{type-label}" --label "{component-label}"
```

Save the PR number as `{PR}`.

**Post to issue:**
```bash
gh issue comment {ISSUE} --body "Draft PR created: #{PR} — starting code review."
```

### Step 3.1: Diff Review

Determine the correct base branch for the diff:
- Small task (single worktree): `origin/main`
- Large task (stacked PRs): `origin/feat/{name}`

```bash
git fetch origin {base}
git diff origin/{base}...HEAD
```

Two-pass review:

**Pass 1 — Critical:**
- Security vulnerabilities (SQL injection, command injection, XSS)
- Data races and concurrency issues
- Logic errors (wrong conditions, off-by-one, null handling)
- CLAUDE.md constraint violations (wrong error handling, missing builders, imperative style)

**Pass 2 — Quality:**
- Dead code or unused imports
- Naming inconsistency with existing codebase
- Missing `///` doc comments on `pub` items
- Test coverage gaps for new functionality
- Code organization (logic in wrong module, missing re-exports)

**Post review findings to PR:**
```bash
gh pr comment {PR} --body "$(cat <<'EOF'
## Code Review — Round {N}

### Critical Issues
{list each issue with file:line and description, or "None found"}

### Quality Issues
{list each issue with file:line and description, or "None found"}

**Verdict:** {Clean — ready to ship / {M} issues to fix}
EOF
)"
```

### Step 3.2: Autonomous Fix Loop

For each issue found, the agent MUST:

1. **Analyze** the root cause — don't just pattern-match the symptom
2. **Search the project** for similar patterns
3. **Research best practices** if the pattern is unfamiliar — use web search
4. **Check constraints** in AGENT.md and CLAUDE.md for the affected area
5. **Implement the fix** — following existing conventions
6. **Verify** the fix compiles and tests pass:
   ```bash
   cargo check -p {crate} && cargo test -p {crate}
   ```

**Do NOT ask the user about any issue that can be resolved through research.**

**Post fix summary to PR:**
```bash
gh pr comment {PR} --body "$(cat <<'EOF'
## Fixes Applied — Round {N}

{for each fix:}
- **{file}:{line}**: {what was wrong} → {what was done}

Pushed fix commit: {short hash}
EOF
)"
```

### Step 3.3: Re-Review

After all fixes are applied:

1. Push fixes and run a full review pass again (both passes)
2. If new issues found → back to Step 3.2
3. **Max 3 rounds** — if still not clean after 3 rounds, escalate to user
4. Clean → proceed to Phase 4

### Step 3.4: Escalation Conditions

Only these situations should be presented to the user:

- 3 review rounds still not clean — show remaining issues and what was tried
- Architecture-level change inconsistent with the approved plan
- Product decision needed (feature trade-off, behavior choice)
- Ambiguous requirement that cannot be resolved from context

Escalation format:
```
## /dev — Escalation Required

**Issue:** {description}
**What I tried:** {research and attempts}
**Options:**
A) {option with trade-off}
B) {option with trade-off}

**My recommendation:** {choice} because {reason}
```

---

## Phase 4: SHIP

### Step 4.1: Final Commit

Ensure all changes are committed with conventional commit format:

```bash
git add {specific files}
git commit -m "$(cat <<'EOF'
{type}({scope}): {description} (#{ISSUE})

{body if needed}

Closes #{ISSUE}

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>
EOF
)"
```

### Step 4.2: Pre-commit Checks

Run the full pre-commit suite:

```bash
just pre-commit  # or: prek run --all-files
```

If checks fail:
- Fix the issues (formatting, clippy warnings, etc.)
- Re-commit
- Retry (max 3 times)

### Step 4.3: Mark PR Ready

Push final changes and update the PR:

```bash
git push
```

Update PR body with the final review log:
```bash
gh pr edit {PR} --body "$(cat <<'EOF'
## Summary

{what was done, 2-3 sentences}

## Type of change

| Type | Label |
|------|-------|
| {type} | `{label}` |

## Component

`{component}`

## Closes

Closes #{ISSUE}

## Test plan

- [x] `cargo check` passes
- [x] `cargo clippy` passes
- [x] `cargo test` passes
- [x] Code review clean (autonomous, {N} rounds)

## Review Log

{summary of review findings and how they were resolved}
EOF
)"

gh pr ready {PR}
```

For large tasks (stacked PRs):
1. Push sub-PRs first (each targeting `feat/{name}`)
2. Push summary PR targeting `main`

### Step 4.4: Wait for CI Green

```bash
gh pr checks {PR} --watch
```

- **CI failure:** analyze the logs, fix in worktree, push again (max 3 attempts)
- Post CI failure analysis as PR comment:
  ```bash
  gh pr comment {PR} --body "CI failure: {analysis and fix applied}"
  ```
- **3 failures:** escalate to user with CI logs

### Step 4.5: Cleanup Reminder

After reporting, remind the user to clean up worktrees after PR is merged:

```bash
git worktree remove .worktrees/issue-{ISSUE}-{name}
git branch -d issue-{ISSUE}-{name}
```

The pipeline does NOT auto-cleanup because the PR hasn't merged yet.

### Step 4.6: Report

Output the final result:

```
## /dev Complete

**Issue:** #{ISSUE}
**PR:** {url}
**Changes:** {summary — crates touched, lines added/removed}
**Review:** {N} rounds, {M} issues found and fixed
**CI:** All checks passed

{one-line summary of what was built}
```

---

## Anti-Patterns

- **Silent analysis** — Do NOT keep investigation conclusions only in conversation context. Every finding goes to GitHub.
- **Bulk dumps** — Do NOT post raw tool output as issue comments. Summarize with context and conclusions.
- **Comment spam** — Do NOT post a comment for every single file read. Group related findings into one comment per logical step.
- **Skipping the trail** — Do NOT skip issue/PR comments "to save time". The audit trail is the point.

## Important Rules

- **GitHub is the work log** — issue comments track investigation and decisions; PR comments track review findings and fixes
- **Never skip the worktree** — all implementation happens in `.worktrees/`, never in the main checkout
- **Never skip review** — even if the change looks trivial, run at least one review pass
- **Draft PR before review** — create the PR as draft before Phase 3 so review comments land on the PR
- **Research before escalating** — the agent must demonstrate it tried to solve the problem
- **Respect CLAUDE.md** — all code must follow project conventions (snafu, bon, functional style, etc.)
- **Conventional commits** — every commit follows the format enforced by the commit-msg hook
- **Labels are mandatory** — every issue and PR must have type + component labels
- **CI must be green** — do not report completion until `gh pr checks --watch` passes
