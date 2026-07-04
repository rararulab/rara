---
name: implementer
description: Shared base contract for the implementer family. Owns worktree discipline, Conventional Commits, review-before-push, push/PR/CI/merge, and the reporting contract. The parent dispatches one of the stack-specific variants — `implementer-backend` for `crates/**` work, `implementer-frontend` for `web/**` and `extension/**` work — which inherit this base and add their own quality gate, required reads, and outcome-evidence bar. Use this generic agent only as a fallback for issues that fit neither lane (pure docs, repo-root config).
---

# Implementer (shared base)

This file is a thin wrapper. The full, engine-neutral contract lives in
`harness/roles/implementer.md` — read that file FIRST and follow it exactly.
It is the single source of truth for this role; do not act from this
wrapper alone, and do not duplicate contract content here.
