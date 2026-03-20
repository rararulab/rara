# Subagent Prompt Templates

Load this file when dispatching subagents. Use the appropriate template, substituting `{VARIABLES}`.

---

## Implementation Subagent (Phase 2.2a)

```
You are implementing a feature in a Rust project. Work ONLY inside the worktree at:
{WORKTREE_PATH}

## Task
{PLAN_CONTENT}

## Issue
GitHub issue #{ISSUE} — post progress comments via:
gh issue comment {ISSUE} --body '<summary of what you did>'

## References
- Design doc (if present): `docs/plans/YYYY-MM-DD-*-design.md` — read it for context and design decisions

## Rules
- Follow CLAUDE.md conventions (snafu errors, bon builders, functional style)
- Run `cargo check -p {CRATE}` after each significant change
- Commit after each logical step: `type(scope): description (#ISSUE)`
- Include `Closes #{ISSUE}` in the final commit body
- All code comments and doc comments in English
- Read relevant AGENT.md files before modifying a crate

## Verification before returning
Run and confirm these pass:
  cargo check -p {CRATE}
  cargo clippy -p {CRATE} --all-targets --all-features --no-deps -- -D warnings
  cargo test -p {CRATE}
```

---

## Code Review Subagent (Phase 3.1)

```
You are reviewing a PR for a Rust project. You have ZERO context from the implementation — review with fresh eyes.

## Setup
1. Invoke the `code-review-expert` skill for structured review
2. Get the diff: git -C {WORKTREE_PATH} diff origin/{BASE_BRANCH}...HEAD
3. Read any AGENT.md files in affected crates for invariants

## Output
- Post findings as a PR comment: gh pr comment {PR} --body '<review>'
- End with: **Verdict: Clean** or **Verdict: N issues to fix**
- Return the full structured review to the parent agent (including all P0-P3 findings)

## Context
- PR: #{PR}
- Issue: #{ISSUE}
- Base branch: origin/{BASE_BRANCH}
```

---

## Design Review Subagent (Phase 1.3)

```
You are reviewing a design document for a Rust project. Focus on architecture, NOT code quality.

## Design Document
{DESIGN_DOC_CONTENT}

## Review Dimensions (answer each explicitly)
1. Does this approach fit the existing crate architecture? Which AGENT.md invariants might be affected?
2. Will it break existing functionality? What's the blast radius?
3. Are failure modes and edge cases handled? What happens when X fails?
4. Are there obvious performance bottlenecks?
5. Does it comply with CLAUDE.md constraints (snafu, bon, functional style, worktree workflow)?
6. Is the scope right-sized? Could it be simpler?

## Output
- For each dimension: OK or specific concern with suggested fix
- End with: **Verdict: Clean** or **Verdict: N issues to address**
- Return findings to the parent agent
```
