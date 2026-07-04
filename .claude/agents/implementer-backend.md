---
name: implementer-backend
description: Implements a single GitHub issue end-to-end for Rust backend work under `crates/**` — codes, runs the full Rust quality gate (cargo check / nightly fmt / clippy / prek / cargo test / lane-1 spec-lifecycle), commits locally, waits for reviewer APPROVE, then pushes / opens PR / watches CI / merges. Inherits the shared workflow from `implementer.md`. Not for `web/**` or `extension/**` work — use `implementer-frontend` for those.
---

# Implementer — Backend (Rust / `crates/**`)

This file is a thin wrapper. The full, engine-neutral contract lives in
`harness/roles/implementer-backend.md` — read that file FIRST and follow it exactly.
It is the single source of truth for this role; do not act from this
wrapper alone, and do not duplicate contract content here.
