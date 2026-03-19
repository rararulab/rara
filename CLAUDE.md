# CLAUDE.md — Rara Development Guide

## Communication
- 用中文与用户交流

## Project Identity

Rara is a self-evolving, developer-first personal proactive agent built in Rust. It uses a kernel-inspired architecture with heartbeat-driven proactive behavior, 3-layer memory, and a skills system.

## Development Workflow

All changes — no matter how small — follow the issue → worktree → PR → merge flow. No exceptions.

@docs/workflow.md — Standard workflow: issue creation, worktree, PR, CI, cleanup
@docs/stacked-prs.md — Stacked PR workflow for large features (> ~400 lines or cross-crate)
@docs/commit-style.md — Conventional Commits format (mandatory)

## Code Quality

@docs/code-comments.md — Doc comment and inline comment guidelines
@docs/agent-md.md — AGENT.md requirements for every crate and major module

## Infrastructure

@docs/database-migrations.md — SQLx migration rules and commands

## Guardrails

@docs/anti-patterns.md — Explicit list of things NOT to do (code, workflow, agent prompts)
