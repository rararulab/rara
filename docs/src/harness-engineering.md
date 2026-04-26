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

## Devtool Commands

All enforcement tools live as subcommands of the Go-based `devtool` CLI under `scripts/`.

### `devtool check-agent-md`

Verifies every crate under `crates/` has an `AGENT.md` file. AGENT.md files are the primary way agents understand a crate's purpose, invariants, and anti-patterns without reading every source file.

```bash
just check-agent-md          # Run standalone
scripts/bin/devtool check-agent-md
```

**Runs in:** PR lint CI (blocking).

### `devtool check-deps`

Validates crate dependency direction against a 7-layer architecture map. Lower-layer crates must not depend on higher-layer crates:

```
Layer 0 (foundation)  → common/*, paths, rara-model, domain/*, rara-api
Layer 1 (domain)      → soul, skills, vault, composio, ...
Layer 2 (core)        → kernel
Layer 3 (subsystems)  → sessions, agents, mcp, ...
Layer 4 (integration) → channels, backend-admin
Layer 5 (application) → app, server
Layer 6 (entry)       → rara-cli
```

```bash
just check-deps               # Run standalone
scripts/bin/devtool check-deps
```

**Runs in:** PR lint CI (blocking).

## CI Integration

| Check | Trigger | Blocking? | Purpose |
|-------|---------|-----------|---------|
| `check-agent-md` | PR lint | Yes | Prevent crates without agent guidelines |
| `check-deps` | PR lint | Yes | Prevent architecture layer violations |
| `cargo clippy -D warnings` | PR lint | Yes | Code quality |
| `cargo +nightly fmt --check` | PR lint | Yes | Formatting consistency |
| `buf lint` | PR lint | Yes | Protobuf schema quality |
| Conventional commits | Pre-commit hook | Yes | Commit message discipline |

## Adding New Checks

When you identify a rule that agents frequently violate:

1. Create a `scripts/internal/<name>/commands.go` subcommand
2. Register it in `scripts/cmd/devtool/main.go`
3. Add a `just` recipe in `justfile`
4. Add a CI job in `.github/workflows/lint.yml`
5. Decide: **blocking** (PR gate) or **tracking** (main-only report)?

The bar for blocking checks is high — they must be deterministic, fast, and have zero false positives. Quality tracking checks can be softer.
