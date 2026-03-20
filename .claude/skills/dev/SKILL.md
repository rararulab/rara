---
name: dev
description: "Autonomous development pipeline for implementing features, fixes, and refactors end-to-end. Triggers: /dev, develop, build feature, implement task, ship PR, new feature, fix bug, add functionality, code change, make this work, add support for, implement this, write the code, build this out, ship this, get this done. One command: requirement → design → implement → review → ship."
---

# /dev — Autonomous Development Pipeline

One command, full cycle: requirement → design → implement → review → ship.

**Iron Law:** (1) Every decision MUST be posted to GitHub — nothing lives only in conversation context. (2) Every phase MUST execute — no shortcuts, no "this is too simple to need X". Use `--quick` for genuinely trivial changes instead of skipping phases ad hoc.

**User intervenes only twice:**
1. After Phase 1: confirm the plan
2. After Phase 4: see the final result

## Parameters

| Flag | Effect | When to use |
|------|--------|-------------|
| (default) | Full pipeline: design → implement → review → ship | Features, non-trivial fixes, refactors |
| `--quick` | Skip Phase 1 (design) and Phase 3.1-3.3 (code review rounds). Still runs: issue → worktree → implement → pre-commit → PR → CI | One-line fixes, typo corrections, config tweaks, doc-only changes |

## Progress Tracking

Use TaskCreate to create one task per phase. Update status as you progress. Do NOT use a copy-paste markdown checklist.

Print to user: "Running /dev pipeline for: {requirement}"

---

## Phase 0: ISSUE CREATION ⛔ BLOCKING

Create the tracking issue **before any analysis begins**. Follow `workflow.md` Step 1 for format.

Note: `--template` flag cannot be used with `--body` — include template fields (### Description, ### Component, ### Alternatives considered) directly in `--body`.

Always include `--label "agent:claude"` plus type + component labels.

Save the issue number as `{ISSUE}`.

---

## Phase 1: DESIGN (skip with `--quick`)

### Step 1.1: Context Gathering

Gather project context silently (no output to user). Answer these questions:

- Is there an existing design doc in `docs/plans/` that addresses this problem?
- Has this been attempted before? `gh issue list --search "{keywords}" --limit 5`
- Which `AGENT.md` files govern the affected crates? What invariants do they declare?
- What existing patterns in the codebase should this change follow?

**Post findings to issue** as a "Context Investigation" comment.

### Step 1.2: Brainstorm & Design Doc

1. What is the core problem being solved?
2. What are 2-3 viable approaches? What are the trade-offs of each?
3. **Autonomously select** the recommended approach — do NOT ask the user
4. Does this fit with: existing architecture, CLAUDE.md constraints, complexity budget?

Write the design doc to a temp file (`/tmp/dev-{ISSUE}-design.md`) using the template from `references/templates.md`. It will be moved to `docs/plans/` in the worktree during Phase 2.

**Post analysis to issue** as a "Design Analysis" comment: approaches considered, selected approach with reasoning, key decisions, implementation steps.

### Step 1.3: Plan Review (autonomous loop)

Dispatch a **general-purpose subagent** to review the design. Load `references/subagent-prompts.md` and use the "Design Review" template, passing the design doc content.

**If issues found:** analyze, revise, re-review (max 2 rounds total).
**Post review result to issue** as a "Plan Review" comment.
**If clean:** proceed to Step 1.4.

### Step 1.4: Present to User ⛔ USER CONFIRMATION REQUIRED

Load `references/templates.md` and use the "Plan Summary" template.

**Wait for user confirmation.** If feedback: revise → re-review → present again.

---

## Phase 2: IMPLEMENT

### Step 2.1: Scale Judgment

Parse the plan and assess task scale:

- **Small/Medium** (≤400 lines, ≤2 crates): single worktree path (Step 2.2a) — **default**
- **Large**: multi-worktree path (Step 2.2b) — use ONLY when ALL of these apply:
  - 3+ truly independent sub-tasks (don't modify the same files, don't depend on each other's output)
  - Estimated >400 lines of change across 3+ crates
  - Parallel execution provides clear benefit

When in doubt, use small task path. Stacked PRs add coordination overhead.

**Post to issue** as an "Implementation Start" comment: scale, path, branch name.

### Step 2.2a: Small/Medium Task — Single Worktree

```bash
git worktree add .worktrees/issue-{ISSUE}-{name} -b issue-{ISSUE}-{name}
```

If design doc exists at `/tmp/dev-{ISSUE}-design.md`, move it into the worktree:
```bash
cp /tmp/dev-{ISSUE}-design.md {WORKTREE}/docs/plans/YYYY-MM-DD-{topic}-design.md
```

Dispatch a **general-purpose subagent** to the worktree. Load `references/subagent-prompts.md` and use the "Implementation Subagent" template.

### Step 2.2b: Large Task — Multi-Worktree Parallel (Stacked PRs)

Follow `stacked-prs.md` for the full stacked PR workflow. Key additions for `/dev`:

- Dispatch each sub-task subagent with `run_in_background: true` for parallel execution
- Use the "Implementation Subagent" template from `references/subagent-prompts.md` for each
- After all subagents complete: verify each worktree compiles, create sub-PRs targeting `feat/{name}`
- **Partial failure:** keep successful worktrees, report failures, escalate to user with options

### Step 2.3: Build Verification

After implementation completes, the **parent agent** verifies in the worktree:

```bash
just pre-commit                    # fmt + clippy + check + test
cargo test -p {crate}              # crate-specific tests
```

If frontend was touched: `cd web && npm run build`

**Post verification result to issue** as a "Build Verification" comment.

**If verification fails:** the parent agent fixes the issues in the worktree and re-verifies (max 3 times). After 3 failures: escalate to user.

---

## Phase 3: REVIEW & FIX (skip review rounds with `--quick`)

### Step 3.0: Create Draft PR

Create a draft PR **before** starting review. Follow `workflow.md` Step 5 for PR format, but create as `--draft`.

Save the PR number as `{PR}`. Post to issue: "Draft PR created: #{PR}"

With `--quick`: create the PR as ready (not draft) and skip to Phase 4.1.

### Step 3.1: Code Review via subagent

Dispatch a **general-purpose subagent** to review the diff. Load `references/subagent-prompts.md` and use the "Code Review Subagent" template.

Base branch: `origin/main` (small task) or `origin/feat/{name}` (stacked PRs).

**Parse the subagent result:**
- **Clean:** proceed to Phase 4
- **Issues found:** proceed to Step 3.2

### Step 3.2: Parent Fixes Issues (main agent, NOT subagent)

The **parent agent** (you) MUST fix issues returned by the reviewer. Do NOT delegate fixes to a subagent.

For each issue:

1. **Analyze** the root cause — don't just pattern-match the symptom
2. **Search the project** for similar patterns
3. **Research best practices** if unfamiliar — use web search
4. **Check constraints** in AGENT.md and CLAUDE.md
5. **Implement the fix** in the worktree
6. **Verify:** `cargo check -p {crate} && cargo test -p {crate}`

**Severity-based handling:**
- **P0-P1:** Fix every one, no exceptions
- **P2:** Fix unless it conflicts with existing patterns
- **P3:** Apply if trivial (< 1 minute). Otherwise note as "acknowledged, deferred" in the fixes comment

**Do NOT ask the user about any issue resolvable through research.**

Post fix summary to PR as a "Fixes Applied" comment: each fix with file:line, what was wrong, what was done.

### Step 3.3: Re-Review (subagent again)

After the parent has fixed issues:

1. Push fixes
2. Dispatch a **new** subagent (same as Step 3.1) — fresh context, no bias
3. Parent fixes new issues (same as Step 3.2)
4. **Max 3 rounds** — if still not clean after 3 rounds, escalate to user
5. Clean → proceed to Phase 4

**Key principle:** subagent reviews, parent fixes, subagent re-reviews. Never the same agent for both.

### Step 3.4: Escalation Conditions

Only escalate to the user for:
- 3 review rounds still not clean
- Architecture-level change inconsistent with approved plan
- Product decision needed (feature trade-off, behavior choice)
- Ambiguous requirement unresolvable from context

Load `references/templates.md` for escalation format.

---

## Phase 4: SHIP

### Step 4.1: Pre-Delivery Checklist ⚠️ REQUIRED

Verify each item before proceeding:

- [ ] All commits follow conventional commit format with `(#ISSUE)`
- [ ] PR body has all sections filled (Summary, Type, Component, Closes, Test plan)
- [ ] No TODO/FIXME/XXX in diff (except intentional, documented ones)
- [ ] Design doc written to `docs/plans/` (skip for `--quick`)
- [ ] All issue comments posted (context, design, review, verification)
- [ ] AGENT.md created/updated if new crate added
- [ ] `just pre-commit` passes clean

### Step 4.2: Mark PR Ready ⛔ BLOCKING

Push final changes and update the PR body: mark test plan items as checked, add review summary. Then:

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

Load `references/templates.md` and use the "Completion Report" template.

Ask user: "Clean up worktree now, or keep for post-merge review?" If yes, run cleanup commands from the template.

---

## Anti-Patterns & Rules

- **Silent analysis** — Do NOT keep investigation conclusions only in conversation context. Every finding goes to GitHub.
- **Bulk dumps** — Do NOT post raw tool output as issue comments. Summarize with context and conclusions.
- **Comment spam** — Do NOT post a comment for every single file read. Group related findings into one comment per logical step. Aim for 3-5 issue comments total, not 10+.
- **Skipping the trail** — Do NOT skip issue/PR comments "to save time". The audit trail is the point.
- **Self-reviewing** — Do NOT review your own implementation in the same context. Always dispatch a fresh subagent.
- **Delegating fixes** — Do NOT dispatch a subagent to fix review findings. The parent agent fixes ALL issues itself, then sends a fresh subagent to re-review.
- **Skipping the worktree** — All implementation happens in `.worktrees/`, never in the main checkout.
- **Skipping review** — Even trivial changes get pre-commit checks. Only `--quick` can skip review rounds.
- **Escalating without research** — Demonstrate you tried to solve the problem before asking the user.
- **Missing labels** — Every issue and PR must have type + component labels.
- **Design doc for trivial changes** — Use `--quick` instead of proposing 2-3 approaches for a typo.
- **Stale worktree base** — Always create worktrees from latest `origin/main` to avoid merge conflicts.
