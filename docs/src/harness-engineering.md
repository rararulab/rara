# Harness Engineering

Rara is built by AI agents (Claude). This page documents how we keep the codebase healthy and agent-friendly through **mechanical enforcement** — automated checks that catch problems before they compound.

> A software engineering team's primary job is no longer to write code, but to design environments, specify intent, and build feedback loops that allow agents to do reliable work.
>
> — [OpenAI, *Harness Engineering*](https://openai.com/index/harness-engineering/)

## Philosophy

Three principles guide our approach:

1. **If a rule exists only in documentation, it will drift.** Every quality rule must have a corresponding automated check.
2. **Waiting is expensive; corrections are cheap.** We use minimal blocking gates at commit time and continuous quality tracking on `main`.
3. **Build the missing capability into the repo.** When an agent struggles, the fix is never "try harder" — it's making the codebase more legible and enforceable.

## Devkit Commands

Enforcement tools are provided by [devkit](https://github.com/rararulab/devkit), a standalone Go CLI. Install with `go install github.com/rararulab/devkit@latest`. Configuration lives in `.devkit.toml` at the repo root.

### `devkit check-agent-md`

Verifies every crate under `crates/` has an `AGENT.md` file. AGENT.md files are the primary way agents understand a crate's purpose, invariants, and anti-patterns without reading every source file.

```bash
just check-agent-md          # Run standalone
devkit check-agent-md
```

**Runs in:** pre-commit hook (blocking).

### `devkit check-deps`

Validates crate dependency direction against a 7-layer architecture map defined in `.devkit.toml`. Lower-layer crates must not depend on higher-layer crates:

```
Layer 0 (foundation)  → common/*, paths, rara-model, domain/*, rara-api
Layer 1 (domain)      → soul, symphony, skills, vault, composio, ...
Layer 2 (core)        → kernel
Layer 3 (subsystems)  → dock, sessions, agents, mcp, ...
Layer 4 (integration) → channels, backend-admin
Layer 5 (application) → app, server
Layer 6 (entry)       → rara-cli
```

```bash
just check-deps               # Run standalone
devkit check-deps
```

**Runs in:** pre-commit hook (blocking).

### `devkit wt`

Interactive worktree manager TUI. Provides selection, bulk cleanup of merged worktrees, pruning, and disk size reporting.

```bash
just wt                       # Launch TUI
devkit wt list                # Non-interactive list
devkit wt clean               # Remove merged worktrees
devkit wt nuke                # Force-remove all except main
```

## CI Integration

| Check | Trigger | Blocking? | Purpose |
|-------|---------|-----------|---------|
| `check-agent-md` | Pre-commit hook | Yes | Prevent crates without agent guidelines |
| `check-deps` | Pre-commit hook | Yes | Prevent architecture layer violations |
| `cargo clippy -D warnings` | PR lint | Yes | Code quality |
| `cargo +nightly fmt --check` | PR lint | Yes | Formatting consistency |
| `buf lint` | PR lint | Yes | Protobuf schema quality |
| Conventional commits | Pre-commit hook | Yes | Commit message discipline |

## Adding New Checks

When you identify a rule that agents frequently violate:

1. Add a new subcommand in [devkit](https://github.com/rararulab/devkit) under `internal/<name>/commands.go`
2. Register it in `main.go`
3. Add a `just` recipe in this repo's `justfile`
4. Optionally add a pre-commit hook in `.pre-commit-config.yaml`
5. Decide: **blocking** (pre-commit gate) or **tracking** (main-only report)?

The bar for blocking checks is high — they must be deterministic, fast, and have zero false positives. Quality tracking checks can be softer.
