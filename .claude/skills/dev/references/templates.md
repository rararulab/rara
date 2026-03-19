# Templates

Load this file when you need to produce structured output for design docs, escalations, or reports.

---

## Design Document Template

Write to `docs/plans/YYYY-MM-DD-{topic}-design.md` in the worktree:

```markdown
# {Title}

## Goal
One sentence: what this change achieves and why.

## Approach
Selected approach with reasoning. Reference alternatives considered (posted in issue comment).

## Affected Crates/Modules
| Crate | What changes | Why |
|-------|-------------|-----|
| {crate} | {description} | {rationale} |

## Key Decisions
- {Decision 1}: {reasoning}
- {Decision 2}: {reasoning}

## Edge Cases
| Scenario | Handling |
|----------|---------|
| {edge case} | {how it's handled} |

## Implementation Steps
1. {step — one commit each}
2. {step}
3. {step}
```

---

## Escalation Format

```markdown
## /dev — Escalation Required

**Issue:** {description}
**What I tried:** {research and attempts}
**Options:**
A) {option with trade-off}
B) {option with trade-off}

**My recommendation:** {choice} because {reason}
```

---

## Completion Report

```markdown
## /dev Complete

**Issue:** #{ISSUE}
**PR:** {url}
**Changes:** {summary — crates touched, lines added/removed}
**Review:** {N} rounds, {M} issues found and fixed
**CI:** All checks passed

{one-line summary of what was built}
```

Cleanup (after PR merged):
```bash
git worktree remove .worktrees/issue-{ISSUE}-{name}
git branch -d issue-{ISSUE}-{name}
```

---

## Plan Summary (presented to user at Phase 1.4)

```markdown
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
