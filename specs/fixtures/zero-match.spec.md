spec: task
name: "fixture-zero-match"
inherits: project
tags: []
---

## Intent

This is a **test fixture, not a real task spec**. It exists to prove that
the spec-lifecycle gate rejects a spec whose `Test:` selector resolves to
zero test functions (the agent-spec <= 0.3.0 false-green documented in
PR 2038 and issue 2165: cargo test prints `0 passed; N filtered out` and
the lifecycle still exits green).

`just spec-selftest` runs the lifecycle gate against this file and asserts
a NON-ZERO exit. If this fixture ever passes the gate, the zero-match
false-green class has returned (upstream regression or guard removal) and
the selftest goes red.

Reproducer this fixture encodes:

1. A lane-1 spec names a test function that does not exist (typo, or the
   test was renamed/deleted after the spec was written — spec drift).
2. `just spec-lifecycle` runs cargo test with that filter.
3. cargo test reports `0 passed; N filtered out`; nothing was executed,
   yet the tool reports the scenario as passed.

## Decisions

- The `Filter:` below intentionally names a test function that must never
  exist anywhere in the workspace. Do NOT "fix" it to point at a real test.
- This file lives in `specs/fixtures/`, outside the `issue-N-*.spec.md`
  naming convention, so tooling that globs real task specs must not pick
  it up.
- Boundaries allow everything: `just spec-selftest` runs in arbitrarily
  dirty worktrees, and the selftest must fail on the zero-match class
  alone — never on an incidental boundary violation.

## Boundaries

### Allowed Changes
- **

### Forbidden
- this/path/must/never/exist/**

## Completion Criteria

Scenario: selector intentionally matches zero tests
  Test:
    Package: rara-paths
    Filter: fixture_zero_match_this_test_function_must_never_exist
  Given a Test selector naming a test function that does not exist in the workspace
  When the spec lifecycle gate runs cargo test with that filter
  Then the runner reports zero executed tests and the gate must fail

## Out of Scope

- Everything. This fixture verifies the verification tooling itself; it
  makes no claim about rara behavior.
