---
name: implementer-frontend
description: Implements a single GitHub issue end-to-end for frontend work under `web/**` or `extension/**` — codes, runs the bun-based quality gate (`bun run build` + TS typecheck + ESLint), self-reviews against the `make-interfaces-feel-better` skill, captures before/after screenshots via real-browser dogfood, commits locally, waits for reviewer APPROVE, then pushes / opens PR / watches CI / merges. Inherits the shared workflow from `implementer.md`. Not for `crates/**` work — use `implementer-backend` for those.
---

# Implementer — Frontend (`web/**`, `extension/**`)

This file is a thin wrapper. The full, engine-neutral contract lives in
`harness/roles/implementer-frontend.md` — read that file FIRST and follow it exactly.
It is the single source of truth for this role; do not act from this
wrapper alone, and do not duplicate contract content here.
