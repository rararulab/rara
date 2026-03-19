---
name: dev
description: "Autonomous development pipeline: requirement → design → implement → review → ship PR."
---

# /dev — Autonomous Development Pipeline

One command, full cycle: requirement → design → implement → review → ship.

**User intervenes only twice:**
1. After Phase 1: confirm the plan
2. After Phase 4: see the final result

**Announce at start:** "Running /dev pipeline for: {requirement}"

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

### Step 1.2: Brainstorm & Design Doc

Based on the gathered context:

1. Analyze the requirement — identify the core problem being solved
2. Propose 2-3 implementation approaches with trade-offs
3. **Autonomously select** the recommended approach — do NOT ask the user
4. Consider: architectural fit, complexity, existing patterns in the codebase, CLAUDE.md constraints
5. Write the design doc to a temporary location (do NOT write to main checkout):

The design doc is drafted in memory during Phase 1 and physically written to `docs/plans/YYYY-MM-DD-{topic}-design.md` inside the worktree created in Phase 2. During Phase 1, the doc content is kept in the conversation context.

The design doc MUST include:
- **Goal:** one sentence
- **Approach:** selected approach with reasoning
- **Affected crates/modules:** list with brief rationale
- **Key decisions:** any non-obvious choices made
- **Edge cases:** identified risks and how they're handled
- **Implementation steps:** numbered list of discrete tasks

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

**If clean:** proceed to Step 1.4

### Step 1.4: Present to User

Output a concise plan summary to the user:

```
## /dev Plan Summary

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

### Step 2.2a: Small Task — Single Worktree

Create the implementation issue using the appropriate GitHub issue template (see workflow.md for template list):

```bash
gh issue create --title "{type}({scope}): {description}" \
  --body "{description with context}" \
  --label "created-by:claude" --label "{type-label}" --label "{component-label}"
```

Note: `--template` flag cannot be used with `--body`. The body MUST follow the template's field structure:

```
### Description
{what and why}

### Component
{crate or area} ({description})

### Alternatives considered
{alternatives or "N/A"}
```

```bash
git worktree add .worktrees/issue-{N}-{name} -b issue-{N}-{name}
```

Dispatch a **subagent** (via the Agent tool) to the worktree with the full plan.

The subagent prompt MUST include:
- The full implementation plan from the design doc
- The worktree path to work in
- Instruction to follow CLAUDE.md conventions
- Instruction to run `cargo check -p {crate}` after each significant change
- Instruction to commit after each logical step with conventional commit messages

### Step 2.2b: Large Task — Multi-Worktree Parallel (Stacked PRs)

```bash
# Create epic issue (with template-compatible body)
gh issue create --title "{type}({scope}): {description}" \
  --body "{description following template structure}" \
  --label "created-by:claude" --label "{type-label}" --label "{component-label}"

# Create feature branch from origin/main (no checkout needed)
git fetch origin main
git branch feat/{name} origin/main
git push -u origin feat/{name}
```

For each independent sub-task:
1. Create a sub-issue referencing the epic
2. Create a worktree branching from `feat/{name}`
3. Dispatch a subagent (via the Agent tool, with `run_in_background: true` for parallel execution)

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

**If verification fails:**
- The implementing subagent self-fixes and retries (max 3 times)
- After 3 failures: escalate to user with error details

---

## Phase 3: REVIEW & FIX

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

### Step 3.2: Autonomous Fix Loop

For each issue found, the agent MUST:

1. **Analyze** the root cause — don't just pattern-match the symptom
2. **Search the project** for similar patterns:
   ```bash
   # How does the rest of the codebase handle this?
   ```
3. **Research best practices** if the pattern is unfamiliar — use web search
4. **Check constraints** in AGENT.md and CLAUDE.md for the affected area
5. **Implement the fix** — following existing conventions
6. **Verify** the fix compiles and tests pass:
   ```bash
   cargo check -p {crate} && cargo test -p {crate}
   ```

**Do NOT ask the user about any issue that can be resolved through research.**

### Step 3.3: Re-Review

After all fixes are applied:

1. Run a full review pass again (both passes)
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

### Step 4.1: Commit

Ensure all changes are committed with conventional commit format:

```bash
git add {specific files}
git commit -m "$(cat <<'EOF'
{type}({scope}): {description} (#{issue-number})

{body if needed}

Closes #{issue-number}

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>
EOF
)"
```

- Use the conventional commit types: feat, fix, refactor, docs, test, chore
- Include `Closes #N` in the body
- Include `Co-Authored-By` trailer

### Step 4.2: Pre-commit Checks

Run the full pre-commit suite:

```bash
just pre-commit  # or: prek run --all-files
```

If checks fail:
- Fix the issues (formatting, clippy warnings, etc.)
- Re-commit
- Retry (max 3 times)

### Step 4.3: Push & PR

```bash
git push -u origin {branch}
```

Create PR using the project template:

```bash
gh pr create --title "{type}({scope}): {description} (#{issue})" --body "$(cat <<'EOF'
## Summary

{what was done, 2-3 sentences}

## Type of change

| Type | Label |
|------|-------|
| {type} | `{label}` |

## Component

`{component}`

## Closes

Closes #{issue}

## Test plan

- [x] `cargo check` passes
- [x] `cargo clippy` passes
- [x] `cargo test` passes
- [x] Code review clean (autonomous, {N} rounds)

## Review Log

{summary of review findings and how they were resolved}
EOF
)" --label "{type-label}" --label "{component-label}"
```

For large tasks (stacked PRs):
1. Push sub-PRs first (each targeting `feat/{name}`)
2. Push summary PR targeting `main`

### Step 4.4: Wait for CI Green

```bash
gh pr checks {PR-number} --watch
```

- **CI failure:** analyze the logs, fix in worktree, push again (max 3 attempts)
- **3 failures:** escalate to user with CI logs

### Step 4.5: Cleanup Reminder

After reporting, remind the user to clean up worktrees after PR is merged:

```bash
git worktree remove .worktrees/issue-{N}-{name}
git branch -d issue-{N}-{name}
```

The pipeline does NOT auto-cleanup because the PR hasn't merged yet. Cleanup happens after merge (per workflow.md Step 6).

### Step 4.6: Report

Output the final result:

```
## /dev Complete

**PR:** {url}
**Changes:** {summary — crates touched, lines added/removed}
**Review:** {N} rounds, {M} issues found and fixed
**CI:** ✓ All checks passed

{one-line summary of what was built}
```

---

## Important Rules

- **Never skip the worktree** — all implementation happens in `.worktrees/`, never in the main checkout
- **Never skip review** — even if the change looks trivial, run at least one review pass
- **Research before escalating** — the agent must demonstrate it tried to solve the problem
- **Respect CLAUDE.md** — all code must follow project conventions (snafu, bon, functional style, etc.)
- **Conventional commits** — every commit follows the format enforced by the commit-msg hook
- **Labels are mandatory** — every issue and PR must have type + component labels
- **CI must be green** — do not report completion until `gh pr checks --watch` passes
