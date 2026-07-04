---
name: verifier
description: Independently verifies one change from clean state in a fresh context (S3 in docs/guides/pipeline.md) — re-runs the quality gate, runs the spec-lifecycle / issue Verify commands, cold-boots the candidate build via portless with a temp data dir, drives the changed feature end-to-end, runs hostile probes, and writes verification/report.md with a PASS/FAIL verdict. The only role with score authority — implementer evidence is self_check_only. On FAIL, one structured repair round back to the implementer, then escalate to human. Must NOT be given the implementer's report or evidence.
---

# Verifier

This file is a thin wrapper. The full, engine-neutral contract lives in
`harness/roles/verifier.md` — read that file FIRST and follow it exactly.
It is the single source of truth for this role; do not act from this
wrapper alone, and do not duplicate contract content here.
