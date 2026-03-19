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

Follow `workflow.md` Step 1 for issue creation format. The body MUST use the template's field structure (### Description, ### Component, ### Alternatives considered). Note: `--template` flag cannot be used with `--body`.

Always include `--label "agent:claude"` plus type + component labels.

Save the issue number as `{ISSUE}` — all subsequent comments reference it.

---

## Phase 1: DESIGN

### Step 1.1: Context Gathering

Gather project context silently (no output to user):

1. Read the project's `CLAUDE.md` and relevant `AGENT.md` files for the affected area
2. Search `docs/plans/*.md` for existing related design documents
3. Search GitHub issues: `gh issue list --search "{keywords}" --limit 5`
4. Read relevant source code in the affected crates/modules

**Post findings to issue** as a "Context Investigation" comment covering: related code, related issues/PRs, existing design docs, key observations.

### Step 1.2: Brainstorm & Design Doc

1. Analyze the requirement — identify the core problem being solved
2. Propose 2-3 implementation approaches with trade-offs
3. **Autonomously select** the recommended approach — do NOT ask the user
4. Consider: architectural fit, complexity, existing patterns, CLAUDE.md constraints

The design doc is drafted in memory during Phase 1 and physically written to `docs/plans/YYYY-MM-DD-{topic}-design.md` inside the worktree created in Phase 2.

The design doc MUST include:
- **Goal:** one sentence
- **Approach:** selected approach with reasoning
- **Affected crates/modules:** list with brief rationale
- **Key decisions:** any non-obvious choices made
- **Edge cases:** identified risks and how they're handled
- **Implementation steps:** numbered list of discrete tasks

**Post analysis to issue** as a "Design Analysis" comment covering: approaches considered, selected approach with reasoning, key decisions, implementation steps.

### Step 1.3: Plan Review (autonomous loop)

Dispatch a **code-reviewer subagent** (via the Agent tool with `subagent_type: "superpowers:code-reviewer"`) to review the design doc.

Review dimensions:
- Architectural soundness — does it fit the existing codebase?
- Compatibility — will it break existing functionality?
- Edge cases — are failure modes handled?
- Performance — any obvious bottlenecks?
- CLAUDE.md compliance

**If issues found:** analyze, revise, re-review (max 2 rounds total).
**Post review result to issue** as a "Plan Review" comment.
**If clean:** proceed to Step 1.4.

### Step 1.4: Present to User

Output a concise plan summary:

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

**Wait for user confirmation.** If feedback: revise → re-review → present again.

---

## Phase 2: IMPLEMENT

### Step 2.1: Scale Judgment

Parse the plan and count independent sub-tasks:

- **Small task** (< 3 independent steps): single worktree path (Step 2.2a)
- **Large task** (3+ independent steps that can run in parallel): multi-worktree path (Step 2.2b)

Independence criteria: tasks that don't modify the same files and don't depend on each other's output.

**Post to issue** as an "Implementation Start" comment: scale, path, branch name.

### Step 2.2a: Small Task — Single Worktree

```bash
git worktree add .worktrees/issue-{ISSUE}-{name} -b issue-{ISSUE}-{name}
```

Dispatch a **subagent** (via the Agent tool) to the worktree with the full plan.

The subagent prompt MUST include:
- The full implementation plan from the design doc
- The worktree path to work in
- The issue number `{ISSUE}` and instruction to post progress comments
- Instruction to follow CLAUDE.md conventions
- Instruction to run `cargo check -p {crate}` after each significant change
- Instruction to commit after each logical step with conventional commit messages

### Step 2.2b: Large Task — Multi-Worktree Parallel (Stacked PRs)

Follow `stacked-prs.md` for branch structure. Create feature branch from `origin/main`:

```bash
git fetch origin main
git branch feat/{name} origin/main
git push -u origin feat/{name}
```

For each independent sub-task:
1. Create a sub-issue referencing `{ISSUE}`
2. Create a worktree branching from `feat/{name}`
3. Dispatch a subagent (via the Agent tool, with `run_in_background: true` for parallel execution)

After all subagents complete:
- Verify each worktree's changes compile
- Each sub-branch creates a PR targeting `feat/{name}` (never merge locally)
- Merge sub-PRs in order via GitHub

**Partial failure:** keep successful worktrees, report failures, escalate to user with options.

### Step 2.3: Build Verification

After implementation completes, verify in the worktree:

```bash
cargo check -p {crate}
cargo clippy -p {crate} --all-targets --all-features --no-deps -- -D warnings
cargo test -p {crate}
```

If frontend was touched: `cd web && npm run build`

**Post verification result to issue** as a "Build Verification" comment.

**If verification fails:** the implementing subagent self-fixes and retries (max 3 times). After 3 failures: escalate to user.

---

## Phase 3: REVIEW & FIX

### Step 3.0: Create Draft PR

Create a draft PR **before** starting review, so review findings can be posted as PR comments. Follow `workflow.md` Step 5 for PR format, but create as `--draft` and add a "Review Log" section with `_Review in progress..._`.

Save the PR number as `{PR}`. Post to issue: "Draft PR created: #{PR} — starting code review."

### Step 3.1: Subagent Review

Dispatch a **code-reviewer subagent** (via the Agent tool with `subagent_type: "superpowers:code-reviewer"`) to review the diff. Subagents start with zero context from the implementation phase — no bias, no assumptions.

Determine the correct base branch:
- Small task: `origin/main`
- Large task (stacked PRs): `origin/feat/{name}`

The subagent prompt MUST include:
- The worktree path and instruction to run `git -C {worktree-path} diff origin/{base}...HEAD`
- PR number `{PR}` and issue number `{ISSUE}`
- Two-pass review instructions:
  - **Pass 1 — Critical:** security vulnerabilities, data races, logic errors, CLAUDE.md constraint violations
  - **Pass 2 — Quality:** dead code, naming inconsistency, missing doc comments, test coverage gaps, code organization
- Instruction to post findings as a PR comment via `gh pr comment {PR} --body '<review>'`
- Required format: `## Code Review`, `### Critical Issues`, `### Quality Issues`, `**Verdict:** Clean` or `N issues to fix`

**Parse the subagent result** to determine the verdict:
- **Clean:** proceed to Phase 4
- **Issues found:** proceed to Step 3.2

### Step 3.2: Autonomous Fix Loop

For each issue found by the reviewer:

1. **Analyze** the root cause — don't just pattern-match the symptom
2. **Search the project** for similar patterns
3. **Research best practices** if unfamiliar — use web search
4. **Check constraints** in AGENT.md and CLAUDE.md
5. **Implement the fix** — following existing conventions
6. **Verify:** `cargo check -p {crate} && cargo test -p {crate}`

**Do NOT ask the user about any issue that can be resolved through research.**

Post fix summary to PR as a "Fixes Applied" comment: each fix with file:line, what was wrong, what was done.

### Step 3.3: Re-Review

After all fixes are applied:

1. Push fixes and dispatch another code-reviewer subagent (same as Step 3.1)
2. If new issues found → back to Step 3.2
3. **Max 3 rounds** — if still not clean after 3 rounds, escalate to user
4. Clean → proceed to Phase 4

### Step 3.4: Escalation Conditions

Only escalate to the user for:
- 3 review rounds still not clean
- Architecture-level change inconsistent with approved plan
- Product decision needed (feature trade-off, behavior choice)
- Ambiguous requirement unresolvable from context

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

### Step 4.1: Final Commit & Pre-commit

Ensure all changes are committed following `commit-style.md`. Then run:

```bash
just pre-commit  # or: prek run --all-files
```

If checks fail: fix, re-commit, retry (max 3 times).

### Step 4.2: Mark PR Ready

Push final changes and update the PR body: mark test plan items as checked, replace "Review Log" section with a summary of review findings and resolutions, then:

```bash
gh pr ready {PR}
```

For large tasks (stacked PRs): push sub-PRs first, then summary PR targeting `main`.

### Step 4.3: Wait for CI Green

```bash
gh pr checks {PR} --watch
```

- **CI failure:** analyze logs, fix in worktree, push again (max 3 attempts). Post CI failure analysis as PR comment.
- **3 failures:** escalate to user with CI logs.

### Step 4.4: Report

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

Remind the user to clean up worktrees after PR is merged:
```bash
git worktree remove .worktrees/issue-{ISSUE}-{name}
git branch -d issue-{ISSUE}-{name}
```

---

## Anti-Patterns

- **Silent analysis** — Do NOT keep investigation conclusions only in conversation context. Every finding goes to GitHub.
- **Bulk dumps** — Do NOT post raw tool output as issue comments. Summarize with context and conclusions.
- **Comment spam** — Do NOT post a comment for every single file read. Group related findings into one comment per logical step.
- **Skipping the trail** — Do NOT skip issue/PR comments "to save time". The audit trail is the point.
- **Self-reviewing** — Do NOT review your own implementation in the same context. Always use a fresh code-reviewer subagent.

## Important Rules

- **GitHub is the work log** — issue comments track investigation and decisions; PR comments track review findings and fixes
- **Never skip the worktree** — all implementation happens in `.worktrees/`, never in the main checkout
- **Never skip review** — even if the change looks trivial, run at least one CLI review pass
- **Draft PR before review** — create the PR as draft before Phase 3 so review comments land on the PR
- **Fresh context for review** — always use a code-reviewer subagent for review, never inline review in the implementing session
- **Research before escalating** — the agent must demonstrate it tried to solve the problem
- **Labels are mandatory** — every issue and PR must have type + component labels
