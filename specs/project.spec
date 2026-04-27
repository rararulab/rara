spec: project
name: "rara"
---

## Intent

rara is a Rust implementation of the personal-AI-agent category — see
`goal.md` at the repo root for the full north star, including what rara is,
what rara is NOT, and the observable signals that define "working".

This project spec defines the technical and process constraints that every
task spec inherits. It is not the place to argue about product direction;
that lives in `goal.md`.

## Constraints

### Style and toolchain

- Errors: `snafu` exclusively in domain and kernel paths. `anyhow` only at
  application boundaries (tools, integrations, bootstrap). Never `thiserror`
  or hand-rolled `impl Error`.
- Construction: `#[derive(bon::Builder)]` for any struct with 3 or more
  fields. No manual `fn new()` for 3+ field structs.
- Async: `#[async_trait]` + `Send + Sync` on async trait definitions.
  `tracing` macros + `#[instrument(skip_all)]` for logging.
- No wildcard imports (`use foo::*`).
- `.expect("context")` over `unwrap()` in non-test code.

### Configuration

- No hardcoded config defaults in Rust code. All static config comes from
  YAML at `~/.config/rara/config.yaml`. See `config.example.yaml` for the
  authoritative key set.
- Mechanism-tuning constants (ring-buffer caps, sweeper intervals, retry
  backoffs) are Rust `const` next to the mechanism, not YAML knobs. Test:
  "would a deploy operator have a real reason to pick a different value?"
  If no → const.

### Code text

- All source comments and doc comments in English.
- New or modified `pub` items require `///` doc comments.
- Inline comments explain *why*, not *what*. Skip comments that restate code.

### Database

- Migrations live in `crates/rara-model/migrations/`, diesel layout
  (`YYYYMMDDHHMMSS_<name>/{up.sql,down.sql}`).
- Never modify already-applied migration files. Schema changes ship as new
  migrations, even when fixing prior ones.

### Process

- Conventional Commits, enforced by local `commit-msg` hook. Format:
  `<type>(<scope>): <description> (#N)` with `Closes #N` in body.
- Allowed types: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`, `ci`,
  `perf`, `style`, `build`, `revert`. Breaking uses `!`.
- Worktree-only edits. The main agent and all subagents never edit files
  on `main` directly. Every change goes through
  `git worktree add .worktrees/issue-N-<slug>`.
- One issue → one PR targeting `main`. No stacked PRs.
- New crates require an `AGENT.md` at crate root before merge.
