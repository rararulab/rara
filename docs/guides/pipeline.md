# Pipeline v2 — Evidence-Chain Development

This document is **normative**: it defines the stages every change moves
through, who may claim what, and where humans sit in the loop. The
day-to-day procedure (worktrees, commits, PR mechanics) lives in
[workflow.md](workflow.md); the machine-readable stage protocol lives in
`harness/stages.toml`; each stage's role contract lives in
`harness/roles/*.md` (with `.claude/agents/*.md` as thin engine wrappers);
the current orchestrator is `.claude/workflows/rara-dev.js`.

## Why an evidence chain

An audit of recent P0 defects (issues 2137, 2136, 2119) found they were
all wiring bugs that shipped because **verification was self-attested by
the implementer**: unit tests on each side of a two-sided feature passed
in isolation, the implementer reported "verified", the reviewer read the
diff — and nobody cold-booted the system and drove the feature end-to-end.
Review (reading the diff) and verification (running the artifact) catch
disjoint failure classes; issue 2137 passed review precisely because
review does not execute.

Pipeline v2's answer is structural, not exhortative:

- **Score authority.** Only the S3 verifier — a fresh context that never
  saw the implementation — may emit `verified`. Implementer evidence is
  `self_check_only` by definition. The verification report's
  `score_authority` field makes this legible in every PR.
- **Every stage produces an artifact** the next stage consumes and a
  check that validates it. No stage advances on prose assurance.

## Stages S0–S7

| Stage | Name | Role | Produces | Advances on |
|-------|------|------|----------|-------------|
| S0 | Spec | `harness/roles/spec-author.md` | issue (lane 2) or issue + `specs/issue-N-<slug>.spec.md` (lane 1) | goal.md gate + prior-art search + reproducer |
| S1 | Plan gate | **human** | acknowledged plan (issue set, lane, variant) | user files / acknowledges the task |
| S2 | Implement | `harness/roles/implementer.md` (backend / frontend variant) | local commits + self-check evidence (`self_check_only`) | stack quality gate green |
| S3 | Verify | `harness/roles/verifier.md` | `verification/report.md` (`score_authority: verifier`) | verdict PASS; `pass_to_fail` = 0 |
| S4 | Review | `harness/roles/reviewer.md` | verdict + P0–P3 findings | APPROVE, no P0/P1 remaining |
| S5 | Ship | `harness/roles/implementer.md` | pushed branch + PR (report path in body) + green required checks | CI green / signoff |
| S6 | Merge | **human** — gate (a) | squash-merge on `main` + cleanup | explicit user confirmation |
| S7 | Retro | human (not yet automated) | probe-derived regression tests; lessons into guides / AGENT.md | — |

S0–S5 are declared in `harness/stages.toml` and orchestrated by
`rara-dev.js`. S6 is a human gate and is never automated. S7 is pipeline
v2 phase 3 — listed for shape, not yet wired.

Ordering is deliberate: **implement → verify → review**. The verifier is
a stage, not a stronger reviewer — verify runs the artifact from clean
state, review reads the diff for design / style / regression decisions.
Verify runs first so the reviewer reads code that is already known to
work, and a verify FAIL never wastes a review round.

## S3 in one paragraph

The verifier receives only the worktree path, the issue number, and the
spec / `Verify:` commands — never the implementer's report. It re-runs
the full quality gate from clean state, runs `just spec-lifecycle`
(lane 1) or the issue's `Verify:` commands (lane 2), cold-boots the
candidate build (`just portless-run`, temp data dir — never a running
instance), drives the changed feature end-to-end including both sides of
any write→read wiring, runs 2–3 hostile probes (CJK input, empty values,
concurrency), and writes `verification/report.md` (`base_sha`,
`head_sha`, `score_authority`, raw command outputs, transition matrix,
verdict). On FAIL: exactly one structured repair round back to the
implementer, then re-verify, then escalate to human. Failing probe
inputs must land as regression tests. Full contract:
`harness/roles/verifier.md`.

## Request-type routing

What S0 produces and what S3 must observe depends on the request type:

| Request type | Entry contract (S0) | S3 expectation shape |
|--------------|--------------------|----------------------|
| **feature** | lane 1 spec with BDD scenarios | new scenarios/tests observed `fail_to_pass` (fail at `base_sha`, pass at `head_sha`); cold-boot drive of the new feature path, both sides of any wiring |
| **bugfix** | lane 1 with a regression test, or lane 2 with a reproducer in `Verify:` | the reproducer fails at `base_sha` and passes at `head_sha`; `pass_to_fail` = 0 |
| **refactor** | lane 2 (behavior-preserving) | no `fail_to_pass` expected; full suite green; `pass_to_fail` = 0 is the whole verdict |
| **perf** | lane 1 or 2 with a measurable target stated in the issue | before/after measurement at `base_sha` vs `head_sha`, same machine, numbers verbatim in the report |
| **chore** | lane 2 with explicit `Verify:` commands | `Verify:` commands re-run from clean state; cold boot only if the change has a runtime surface (state explicitly if not) |

If a request fits none of these rows, S0 must say so and route it back
to the user rather than force a shape.

## Parallel-run rules

Multiple issues run the pipeline concurrently (one worktree, branch, and
PR each — see "Parallel execution" in workflow.md). The isolation rules:

- **Ports via portless.** Every cold boot goes through `just
  portless-run`; portless assigns a stable per-worktree URL and injects
  `PORT`. Never hardcode `:25555` / `:5173` in a parallel run.
- **Temp data dir per run.** A verifier boot never points at your real
  config / DB paths, and never at another run's temp dir.
- **`base_sha` pinning.** Every verification report pins `base_sha` and
  `head_sha`. A rebase (or any new commit) invalidates the verdict —
  re-verify after rebasing onto a moved `origin/main`; a stale PASS
  must never ride into S5.
- **Boundaries-glob as lock.** Two in-flight issues must not have
  overlapping `Boundaries.Allowed` globs. Overlap means they are not
  independent — S0 must serialize them (or merge them into one issue)
  instead of letting them race on the same files.
- **D-numbering by issue.** Every per-run artifact — worktree, branch,
  portless name, temp data dir, report — is keyed by the issue number
  (`issue-N-<slug>`), never by timestamp or random suffix, so every
  piece of evidence is attributable to exactly one dispatch.

## Human-in-the-loop

Exactly **two routine touchpoints**:

1. **File the task** (S1) — the user's request plus acknowledgment of
   the plan is what unlocks auto-chaining through S2–S5.
2. **Merge the PR** (S6) — gate (a) in workflow.md; always confirmed,
   even with green CI, verifier PASS, and reviewer APPROVE.

And exactly **three escalations** that interrupt the chain:

1. **Stuck loops** — S3 still FAIL after its one repair round, or S4
   not APPROVE after 3 rounds. The pipeline stops and reports; it does
   not grant itself more rounds.
2. **Destructive git operations** — gate (b) in workflow.md
   (`reset --hard`, force-push, shared-branch deletion).
3. **Production intervention** — gate (c) in workflow.md (restarting or
   mutating the shared production instance).

Everything else default-continues; the gate list is closed (see
"Auto-chaining" in workflow.md).
