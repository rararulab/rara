# CLAUDE.md — Rara Development Guide

## Communication
- 用中文与用户交流

## Project Philosophy

Rara is a kernel-inspired personal AI agent in Rust — self-evolving, developer-first,
with heartbeat-driven proactive behavior, 3-layer memory, and a skills system.

Design ethos: **"Boring Technology"** (Dan McKinley) meets **Linux kernel discipline**.
Proven Rust patterns over novel abstractions. Explicit resource ownership, clear subsystem
boundaries, no hidden magic. When in doubt, choose the solution a senior kernel developer
would find unsurprising.

## Style Anchors

Rust style triangulated from three voices — each covers a different blind spot:

- **BurntSushi** (Andrew Gallant): error ergonomics via `snafu`, CLI patterns, exhaustive matching, documentation-first design
- **dtolnay** (David Tolnay): API minimalism, derive-macro philosophy (`serde`, `bon`), "if it compiles it works" surface area
- **Niko Matsakis**: ownership-first API design, type safety as a feature, making invalid states unrepresentable

When these anchors conflict, prefer: safety (Niko) > ergonomics (BurntSushi) > minimalism (dtolnay).

## External Reality

These artifacts are authoritative — your work is accountable to them, not just to the user:

- `.pre-commit-config.yaml` — code quality gate (clippy, fmt, doc warnings)
- `.github/ISSUE_TEMPLATE/` — issue structure and required fields
- `.github/pull_request_template.md` — PR structure
- `config.example.yaml` — all config keys; no hardcoded defaults in Rust
- `AGENT.md` files per crate — architecture invariants and anti-patterns

## Development Workflow

All changes — no matter how small — follow the issue → worktree → PR → merge flow. No exceptions.

@docs/guides/workflow.md
@docs/guides/commit-style.md

## Code Quality

@docs/guides/rust-style.md
@docs/guides/code-comments.md
@docs/guides/agent-md.md

## Infrastructure

@docs/guides/database-migrations.md
@docs/guides/debug.md

## Guardrails

@docs/guides/anti-patterns.md

## Anti-sycophancy

If the user's architectural request conflicts with the style anchors above, an existing
`AGENT.md`, or the pre-commit quality gate, say so directly. Quote the specific conflict.
Do not soften disagreement with hedge phrases like "you might want to consider" —
state the conflict, explain why, and propose the alternative.
