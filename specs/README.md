# specs/ — Task Contracts

This directory holds [`agent-spec`](https://github.com/ZhangHanDong/agent-spec)
Task Contracts for rara work. Read `goal.md` first, then this file.

## Two lanes

Not every change needs a `.spec.md`. Pick the right lane.

### Lane 1 — Spec-driven (feature, bugfix, anything with testable behavior)

Use lane 1 when **at least one acceptance criterion can be bound to a real
test function** that fails before the change and passes after. Examples:

- New API endpoint with request/response behavior
- Bug fix where you can write a failing reproducer
- New tape-memory behavior with observable recall semantics
- Refactor that must preserve a documented contract (with parity tests)

Lane 1 flow:

1. `spec-author` writes `specs/issue-N-<slug>.spec.md` inheriting `project`.
2. `spec-author` creates the GitHub issue, referencing the spec file.
3. `implementer` reads the spec, implements, runs `agent-spec lifecycle`
   locally, commits inside the worktree, **does not push**.
4. `reviewer` reads the worktree diff plus the spec; verifies the BDD
   scenarios pass; produces a verdict.
5. On APPROVE: implementer pushes, opens the PR, watches CI, merges.

### Lane 2 — Lightweight chore (structural, cleanup, CI, rename, config)

Use lane 2 when there is **no test function that meaningfully verifies
"done"**. Examples:

- Deleting a workflow file
- Renaming a directory
- Updating dependencies
- Editing documentation
- Restructuring a module without behavior change
- This very PR (landing the harness)

Lane 2 flow:

1. `spec-author` writes the GitHub issue body directly with Intent + prior
   art + decisions + boundaries — same content shape as a Task Contract,
   minus BDD scenarios. No `specs/*.spec.md` file is created.
2. `implementer` reads the issue, implements, runs `cargo check` /
   `prek run --all-files`, commits, **does not push**.
3. `reviewer` reads the worktree diff plus the issue body.
4. On APPROVE: implementer pushes, opens the PR, watches CI, merges.

## How spec-author chooses the lane

A single question: **"Can I write at least one `Test:` selector that binds
to a real test function that meaningfully verifies the outcome?"**

- Yes → lane 1.
- No → lane 2.

If unsure, lane 2 — overhead-on-the-side-of-less. Lane 1's value is the BDD
binding. Without that binding, lane 1 produces ceremony, not safety.

## Test selectors MUST resolve to real tests

Every lane-1 spec's `Test:` selectors MUST resolve to **at least one real
test function** in the current codebase. A lifecycle run whose test output
shows `filtered out` with **0 passed** (cargo ran `0 tests` for that
selector) is a **FAIL, never a pass** — nothing was executed, so nothing
was verified.

Background: agent-spec (<= 0.3.0) reports such scenarios as green — the
zero-match false-green documented in PR #2038 and issue #2165, reported
upstream. `just spec-lifecycle` therefore routes through
`scripts/spec-lifecycle-guard.sh`, which inspects the per-scenario runner
output and exits non-zero on the zero-match signature regardless of
agent-spec's own verdict. Do not call `agent-spec lifecycle` directly.

Self-check: `just spec-selftest` asserts the gate rejects
`specs/fixtures/zero-match.spec.md` (a fixture whose selector intentionally
matches zero tests). If the selftest goes red, the false-green class has
returned. `just spec-drift-sweep` checks all existing task specs for
selectors that no longer resolve (spec drift).

## Naming

- Task spec files: `specs/issue-<N>-<slug>.spec.md`
- Inherits clause: `inherits: project` (the constraints in `project.spec`)
- Fixtures for the verification tooling itself: `specs/fixtures/*.spec.md`
  — intentionally outside the `issue-N-` convention so tooling that globs
  real task specs never picks them up

## Project-level constraints

`project.spec` carries the toolchain and process rules that every task
inherits. It is not the place to argue about product direction; that is
`goal.md`.

## Why we adopted this

Discussed 2026-04-27. The triggering case was PR 1941, which added a
real-LLM e2e workflow on every push to `main`. Its assertions
(`saw_anchor`, `read_file_calls >= 9`) tested OpenAI's instruction-following,
not rara's tape-memory code. Root cause: the gap between "user request"
and "issue body" had no contract — vague requests became intervention-form
issues, which downstream agents implemented faithfully but uselessly.

The lanes plus `spec-author` close that gap. The `agent-spec` tool gives
us the BDD machinery for lane 1; lane 2 stays in plain GitHub issues
because adding ceremony to deletes does not improve them.
