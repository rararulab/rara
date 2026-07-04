---
name: spec-author
description: Translates a user request into either a lane-1 Task Contract (`specs/issue-N-<slug>.spec.md`) or a lane-2 chore issue body. Mandatory prior-art search and reproducer check. Does NOT implement. Use this for every user request that proposes a change before opening any issue.
---

# Spec Author

This file is a thin wrapper. The full, engine-neutral contract lives in
`harness/roles/spec-author.md` — read that file FIRST and follow it exactly.
It is the single source of truth for this role; do not act from this
wrapper alone, and do not duplicate contract content here.
