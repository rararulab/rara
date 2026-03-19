---
name: dev
description: "Autonomous development pipeline for implementing features, fixes, and refactors end-to-end. Triggers: /dev, develop, build feature, implement task, ship PR, new feature, fix bug, add functionality, code change. One command: requirement → design → implement → review → ship."
---

# /dev — Autonomous Development Pipeline

One command, full cycle: requirement → design → implement → review → ship.

**Iron Law:** Every decision and finding MUST be posted to GitHub (issue/PR comments) — nothing lives only in conversation context.

**User intervenes only twice:**
1. After Phase 1: confirm the plan
2. After Phase 4: see the final result

Print to user: "Running /dev pipeline for: {requirement}"

## Progress Checklist

Copy and track progress:

```
- [ ] Phase 0: Issue created ⛔ BLOCKING
- [ ] Phase 1.1: Context gathered
- [ ] Phase 1.2: Design doc drafted
- [ ] Phase 1.3: Plan reviewed
- [ ] Phase 1.4: User confirmed ⛔ BLOCKING
- [ ] Phase 2.1: Scale judged
- [ ] Phase 2.2: Implementation complete
- [ ] Phase 2.3: Build verified
- [ ] Phase 3.0: Draft PR created
- [ ] Phase 3.1: Code review complete
- [ ] Phase 3.2–3.3: Fixes applied & re-reviewed (if needed)
- [ ] Phase 4.1: Pre-commit passed
- [ ] Phase 4.2: PR marked ready
- [ ] Phase 4.3: CI green ⛔ BLOCKING
- [ ] Phase 4.4: Reported to user
```

---

## Phase 0: ISSUE CREATION ⛔ BLOCKING

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

Draft the design doc content in conversation context. It will be physically written to `docs/plans/YYYY-MM-DD-{topic}-design.md` inside the worktree created in Phase 2.

The design doc MUST include:
- **Goal:** one sentence
- **Approach:** selected approach with reasoning
- **Affected crates/modules:** list with brief rationale
- **Key decisions:** any non-obvious choices made
- **Edge cases:** identified risks and how they're handled
- **Implementation steps:** numbered list of discrete tasks

**Post analysis to issue** as a "Design Analysis" comment covering: approaches considered, selected approach with reasoning, key decisions, implementation steps.

### Step 1.3: Plan Review (autonomous loop)

Dispatch a **general-purpose subagent** (via the Agent tool) to review the design. Pass the full design doc content in the subagent prompt and instruct it to invoke the `code-review-expert` skill for structured review.

The subagent prompt MUST include:
- The full design doc content (since the file doesn't exist yet)
- Instruction: "Invoke the `code-review-expert` skill to review this design"
- Review dimensions: architectural soundness, compatibility, edge cases, performance, CLAUDE.md compliance

**If issues found:** analyze, revise, re-review (max 2 rounds total).
**Post review result to issue** as a "Plan Review" comment.
**If clean:** proceed to Step 1.4.

### Step 1.4: Present to User ⛔ USER CONFIRMATION REQUIRED

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

Parse the plan and assess task scale:

- **Small task**: single worktree path (Step 2.2a) — default for most tasks
- **Large task**: multi-worktree path (Step 2.2b) — use ONLY when ALL of these apply:
  - 3+ truly independent sub-tasks (don't modify the same files, don't depend on each other's output)
  - Estimated >400 lines of change across 3+ crates
  - Parallel execution provides clear benefit

When in doubt, use small task path. Stacked PRs add coordination overhead.

**Post to issue** as an "Implementation Start" comment: scale, path, branch name.

### Step 2.2a: Small Task — Single Worktree

```bash
git worktree add .worktrees/issue-{ISSUE}-{name} -b issue-{ISSUE}-{name}
```

Dispatch a **general-purpose subagent** (via the Agent tool) to the worktree with the full plan.

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
3. Dispatch a general-purpose subagent (via the Agent tool, with `run_in_background: true` for parallel execution)

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

### Step 3.1: Code Review via `code-review-expert` (subagent)

Dispatch a **general-purpose subagent** (via the Agent tool) to review the diff. Subagents start with zero context from the implementation phase — no bias, no assumptions.

Determine the correct base branch:
- Small task: `origin/main`
- Large task (stacked PRs): `origin/feat/{name}`

The subagent prompt MUST include:
- Instruction: "Invoke the `code-review-expert` skill to perform a structured code review"
- The worktree path and instruction to run `git -C {worktree-path} diff origin/{base}...HEAD`
- PR number `{PR}` and issue number `{ISSUE}`
- Instruction to post findings as a PR comment via `gh pr comment {PR} --body '<review>'`
- Instruction to include a verdict: `**Verdict:** Clean` or `**Verdict:** N issues to fix`
- Instruction to return the full structured review result to the parent agent

**Parse the subagent result** to determine the verdict:
- **Clean:** proceed to Phase 4
- **Issues found:** proceed to Step 3.2

### Step 3.2: Parent Fixes ALL Issues (main agent, NOT subagent)

The **parent agent** (you) MUST fix every issue returned by the reviewer. Do NOT delegate fixes to a subagent — you have full context of the implementation and the review.

For each issue:

1. **Analyze** the root cause — don't just pattern-match the symptom
2. **Search the project** for similar patterns
3. **Research best practices** if unfamiliar — use web search
4. **Check constraints** in AGENT.md and CLAUDE.md
5. **Implement the fix** in the worktree — following existing conventions
6. **Verify:** `cargo check -p {crate} && cargo test -p {crate}`

**Do NOT ask the user about any issue that can be resolved through research.**
**Do NOT skip any issue** — address every P0, P1, P2, and P3 finding.

Post fix summary to PR as a "Fixes Applied" comment: each fix with file:line, what was wrong, what was done.

### Step 3.3: Re-Review (subagent again)

After the parent has fixed ALL issues:

1. Push fixes
2. Dispatch a **new** subagent with `code-review-expert` (same as Step 3.1) — fresh context, no bias
3. Parent fixes all new issues (same as Step 3.2)
4. **Max 3 rounds** — if still not clean after 3 rounds, escalate to user
5. Clean → proceed to Phase 4

**Key principle:** subagent reviews, parent fixes, subagent re-reviews. Never the same agent for both.

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

### Step 4.2: Mark PR Ready ⛔ BLOCKING

Push final changes and update the PR body: mark test plan items as checked, replace "Review Log" section with a summary of review findings and resolutions, then:

```bash
gh pr ready {PR}
```

For large tasks (stacked PRs): push sub-PRs first, then summary PR targeting `main`.

### Step 4.3: Wait for CI Green ⛔ BLOCKING

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

## Anti-Patterns & Rules

- **Silent analysis** — Do NOT keep investigation conclusions only in conversation context. Every finding goes to GitHub.
- **Bulk dumps** — Do NOT post raw tool output as issue comments. Summarize with context and conclusions.
- **Comment spam** — Do NOT post a comment for every single file read. Group related findings into one comment per logical step.
- **Skipping the trail** — Do NOT skip issue/PR comments "to save time". The audit trail is the point.
- **Self-reviewing** — Do NOT review your own implementation in the same context. Always dispatch a fresh subagent with `code-review-expert`.
- **Delegating fixes** — Do NOT dispatch a subagent to fix review findings. The parent agent fixes ALL issues itself, then sends a fresh subagent to re-review.
- **Skipping the worktree** — All implementation happens in `.worktrees/`, never in the main checkout.
- **Skipping review** — Even trivial changes get at least one `code-review-expert` pass.
- **Escalating without research** — Demonstrate you tried to solve the problem before asking the user.
- **Missing labels** — Every issue and PR must have type + component labels.
