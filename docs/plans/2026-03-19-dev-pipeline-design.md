# `/dev` Pipeline — Autonomous Development Skill

> **Source of truth:** `.claude/skills/dev/SKILL.md` — this design doc captures the rationale; the skill file is the authoritative spec.

## Overview

A single slash command `/dev "requirement"` that runs the full development pipeline autonomously: from brainstorming to shipped PR. The user intervenes only twice — confirming the plan, and seeing the final result.

## Design Principles

- **Autonomous by default** — the agent researches, decides, and fixes on its own
- **Escalate, don't delegate** — only ask the user when a product decision is needed or after 3 failed attempts
- **Ralph-style review** — when issues are found, research best practices and fix them; don't throw problems back to the user
- **Respect existing constraints** — CLAUDE.md rules (snafu, bon builder, functional style, worktree workflow) are already global; the skill doesn't duplicate them

## Pipeline Phases

```
/dev "requirement description"
  │
  ├─ Phase 1: DESIGN ──── brainstorm → plan → review plan → revise → present to user
  │                                                                    ↑ interaction 1
  ├─ Phase 2: IMPLEMENT ─ judge scale → issue → worktree → subagent → build verify
  │
  ├─ Phase 3: REVIEW ──── diff review → autonomous research & fix → re-review (≤3 rounds)
  │
  └─ Phase 4: SHIP ────── commit → PR → CI green → report result
                                                    ↑ interaction 2
```

---

## Phase 1: DESIGN

### Step 1.1: Context Gathering

- Read project structure, relevant AGENT.md files, related crate code
- Search `docs/plans/` for existing related design documents
- Search GitHub issues for related discussions

### Step 1.2: Brainstorm

- Analyze the requirement, propose 2-3 approaches with trade-offs
- Autonomously select the recommended approach (do not ask the user)
- Generate design doc → `docs/plans/YYYY-MM-DD-{topic}-design.md`

### Step 1.3: Plan Review (autonomous loop)

- Invoke code-review-expert to review the design doc
- Review dimensions:
  - Architectural soundness
  - Compatibility with existing code
  - Edge cases and error handling
  - Performance implications
  - Compliance with CLAUDE.md constraints
- If issues found → research autonomously (read code, search docs) → revise plan
- Re-review until clean (max 2 rounds)

### Step 1.4: Present to User

- Output plan summary: goal, approach, key decisions, affected crates
- Wait for user reply: "ok" or revision feedback
- If feedback → revise and re-review → present again

---

## Phase 2: IMPLEMENT

### Step 2.1: Scale Judgment

- Parse the plan, identify independent sub-tasks
- Small task (< 3 independent steps) → single worktree
- Large task (3+ independent steps) → multi-worktree parallel (stacked PRs)

### Step 2.2a: Small Task Path

```bash
# Create issue with template + labels
gh issue create --template {template} --title "{title}" --label "created-by:claude" --label "{component}"

# Create worktree
git worktree add .worktrees/issue-{N}-{name} -b issue-{N}-{name}

# Dispatch subagent to implement in worktree
# Subagent follows the plan step by step

# Verify in worktree
cargo check -p {crate}
cargo clippy -p {crate} -- -D warnings
cargo test -p {crate}
```

### Step 2.2b: Large Task Path (Stacked PRs)

```bash
# Create epic issue
gh issue create --template feature_request.yml --title "feat(scope): {description}"

# Create feature branch
git checkout main && git pull
git branch feat/{name} main
git push -u origin feat/{name}

# For each independent sub-task:
#   - Create sub-issue referencing epic
#   - Create worktree branching from feat/{name}
#   - Dispatch subagent (parallel execution)
#
# After all complete → merge into feat/{name}
```

### Step 2.3: Build Verification

- `cargo check -p {crate}`
- `cargo clippy -p {crate} --all-targets --all-features --no-deps -- -D warnings`
- `cargo test -p {crate}`
- Frontend changes: `cd web && npm run build`
- Failure → subagent self-fixes → retry (max 3 times)

---

## Phase 3: REVIEW & FIX

### Step 3.1: Diff Review

```bash
git diff origin/<base> -- # full diff against base branch
```

Two-pass review:
- **Pass 1 (Critical):** security vulnerabilities, data races, logic errors, CLAUDE.md constraint violations
- **Pass 2 (Quality):** dead code, naming inconsistency, missing doc comments, test coverage gaps

### Step 3.2: Autonomous Fix Loop

For each issue found:

1. Analyze the root cause
2. Search project code for similar patterns / conventions
3. Web search for best practices if needed
4. Check AGENT.md / CLAUDE.md constraints
5. Implement the fix
6. Verify fix: `cargo check` + `cargo test`

### Step 3.3: Re-Review

- After all fixes, run a full review pass again
- Still has issues → back to Step 3.2
- Max 3 rounds; if still not clean → escalate to user
- Clean → proceed to Phase 4

### Step 3.4: Escalation Conditions

Only these situations prompt the user:
- 3 review rounds still not clean
- Architecture-level change inconsistent with the plan
- Product decision needed (feature trade-off, behavior choice)

---

## Phase 4: SHIP

### Step 4.1: Commit

- Conventional commit format: `type(scope): description (#N)`
- Commit body includes `Closes #N`
- Large tasks: each sub-PR gets independent commits

### Step 4.2: Push & PR

```bash
git push -u origin {branch}
gh pr create --title "{title}" --body "..." --label "{type}" --label "{component}"
```

- Uses project PR template (`.github/pull_request_template.md`)
- Large tasks: push sub-PRs first → then summary PR targeting main

### Step 4.3: Wait for CI Green

```bash
gh pr checks {N} --watch
```

- CI failure → analyze logs → fix → re-push (max 3 times)
- 3 failures → escalate to user

### Step 4.4: Report

Output:
- PR URL
- Change summary (what was done, which crates touched)
- Review rounds log (issues found + how they were resolved)
- CI status

---

## Key Differences from gstack

| Aspect | gstack `/ship` | rara `/dev` |
|--------|---------------|-------------|
| Scope | Review + ship only | Full pipeline: design → implement → review → ship |
| Review issues | ASK items → ask user | Autonomous research + fix; escalate only if stuck |
| Isolation | Direct branch work | Worktree isolation (per CLAUDE.md) |
| Scale handling | Single branch | Auto-judge: single vs stacked PRs |
| Chaining | File artifacts + inline | Single skill, phased execution |
| User interaction | Multiple touchpoints | Only 2: plan confirm + final report |

## Implementation Notes

- This skill will be a Claude Code slash command (SKILL.md format)
- It orchestrates existing infrastructure via Agent tool subagents (general-purpose type)
- CLAUDE.md constraints (snafu, bon, functional style, commit style) are already global — not duplicated in the skill
- The skill itself is a Markdown prompt, not a code pipeline
- Design doc is drafted in conversation context during Phase 1, physically written to worktree in Phase 2

### Dependency Skills (referenced internally)

| Skill / Tool | Used in | Purpose |
|--------------|---------|---------|
| Agent (general-purpose) | Phase 1.3, 2.2 | Dispatch review and implementation subagents |
| Agent (Explore) | Phase 1.1 | Codebase context gathering |
| Glob / Grep / Read | Phase 1.1, 3.2 | Search files and code patterns |
| Bash | Phase 2-4 | git, gh, cargo commands |
| WebSearch | Phase 3.2 | Research best practices for review fixes |
