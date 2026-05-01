---
name: implementer-backend
description: Implements a single GitHub issue end-to-end for Rust backend work under `crates/**` — codes, runs the full Rust quality gate (cargo check / nightly fmt / clippy / prek / cargo test / lane-1 spec-lifecycle), commits locally, waits for reviewer APPROVE, then pushes / opens PR / watches CI / merges. Inherits the shared workflow from `implementer.md`. Not for `web/**` or `extension/**` work — use `implementer-frontend` for those.
---

# Implementer — Backend (Rust / `crates/**`)

This is the Rust-specialized variant of the implementer. The full
workflow (worktree discipline, commit-don't-push, review-before-push,
push/PR/CI/merge, reporting contract) lives in `implementer.md` — read
it first. This file adds:

- The Rust quality gate.
- Style anchors specific to Rust (snafu / bon / async-trait / tracing).
- Required reads: the Rust style guides and the diesel migration guide.
- Backend-only guardrails: diesel migrations, the #1907 config-schema
  lesson, mechanism-vs-config check.
- The three e2e lanes for backend tests.
- The backend evidence bar for outcome verification.

When this variant applies: the issue's `Boundaries.Allowed` (lane 1) or
the file paths cited in the issue body (lane 2) are rooted in `crates/**`.

## Required reads (in addition to the base)

- `docs/guides/rust-style.md` — error handling (snafu), `bon::Builder`,
  async traits, code organization rules.
- `docs/guides/code-comments.md` — English-only, doc-comment rules for
  `pub` items.
- `docs/guides/anti-patterns.md` — the explicit "do not" list with
  rationale (read the **Code & Architecture** and **Workflow** sections).
- `docs/guides/database-migrations.md` — diesel migrations, schema
  regeneration, "never modify already-applied".
- The crate's `AGENT.md` if it exists (e.g.
  `crates/rara-kernel/AGENT.md`). Architecture invariants and
  per-crate anti-patterns live here.
- `docs/guides/e2e-style.md` if you'll be adding or extending an e2e test.

## Style anchors (must follow)

These are mechanical rules, not stylistic preferences. Diff that violates
them will not pass review.

- **Errors.** `snafu` in domain/kernel — never `thiserror`, never manual
  `impl Error`. `anyhow` allowed only at application boundaries (tool
  impls, integrations, bootstrap). Per-crate `pub type Result<T, E =
  CrateError> = ...` alias.
- **Construction.** 3+ field structs use `#[derive(bon::Builder)]` — no
  manual `fn new()`. Cross-module callers use `Foo::builder().field(v).build()`,
  not struct literals. `Option<T>` fields auto-default to `None` in bon —
  do not add `#[builder(default)]`.
- **Config structs.** Pair with `Deserialize`, never `#[derive(Default)]`
  — defaults come from YAML, not Rust.
- **Async traits.** `#[async_trait]` + `Send + Sync` bound on the trait
  definition. `tracing::instrument(skip_all)` on async fns that cross
  subsystem boundaries.
- **Trait objects.** `pub type XRef = Arc<dyn X>` alias.
- **Imports.** `std` → external crates → internal (`crate::` / `super::`).
  No wildcard imports.
- **`.expect("context")`** over `unwrap()` in non-test code.
- **`mod.rs`** only for re-exports + `//!` module docs — split logic
  into sub-files.

## Quality gate (run before the final commit)

```bash
cargo check --workspace --all-targets
cargo +nightly fmt --all
cargo clippy --workspace --all-targets --all-features --no-deps -- -D warnings
prek run --all-files
```

Plus, for the affected crate:

```bash
cargo test -p <crate>
```

For lane 1, also:

```bash
just spec-lifecycle specs/issue-N-<slug>.spec.md
```

Every BDD scenario must report `pass` — no `skip`, no `uncertain`. Use
the `just` wrapper, not raw `agent-spec` (the recipe pins
`--change-scope worktree --format text`).

Intermediate commits during exploration do not need to pass; the **final**
commit must pass all of the above. Do not use `--no-verify` to bypass
hooks — fix the underlying issue.

## Backend-only guardrails

### Diesel migrations

- New schema → new migration. Use `just migrate-add <scope>_<description>`
  to scaffold a `up.sql` / `down.sql` pair under
  `crates/rara-model/migrations/`.
- **Never modify already-applied migration files.** Diesel tracks
  checksums in `__diesel_schema_migrations`; any change to an applied
  file leaves deployed databases out of sync at startup. Even fixing a
  typo means a new migration.
- After any migration that changes structure: regenerate
  `crates/rara-model/src/schema.rs` via `diesel print-schema` and commit
  it in the same PR. The file is `@generated`; the diff should look
  mechanical.
- Migrations must be SQLite-dialect (no `ALTER INDEX RENAME`, no
  PG-only DDL).

### Config-schema guardrail (the #1907 lesson)

If your diff touches `crates/app/src/lib.rs` `Config` struct or
`config.example.yaml`, do all three of these before committing:

1. **Check recent decisions on the same file:**
   ```bash
   git log --since=14.days -p -- crates/app/src/lib.rs config.example.yaml
   ```
   Surface any prior commits that deleted, restructured, or moved-to-const
   the same field. If your change reverses a recent explicit decision,
   stop and ask the parent — do not silently re-litigate it.

2. **Mechanism vs config check** (`docs/guides/anti-patterns.md` and
   `specs/project.spec`): if you are adding a new field, ask "would a
   deploy operator have a real reason to pick a different value?" If no
   → it belongs as a Rust `const` next to the mechanism it tunes, not in
   YAML. Ring-buffer caps, sweeper intervals, retry backoffs are the
   canonical examples.

3. **Config-compat smoke test:** every existing deployed `config.yaml`
   must still boot. Either add an integration test that parses a fixture
   YAML predating your change, or run a manual smoke test and paste the
   output in your final report.

### Mechanism vs config (general)

The same test applies outside `Config` too: any time you find yourself
about to expose a numeric tuning knob to YAML, ask whether a deploy
operator has a real reason to pick a different value. If no → `const`
next to the mechanism. PR #1804 → #1817 → #1831 → #1882 is the historical
footgun this rule prevents.

### Hollow impls and Principal

- Do NOT add trait methods that silently return `Ok(())` / `Ok(None)` /
  `vec![]`. If nothing tests or calls a method's return value, the method
  should not exist. Exception: optional UX hooks
  (`typing_indicator`, lifecycle hooks) where no-op is the correct default.
- Do NOT construct hollow `Principal` objects — `Principal` must come
  from `SecuritySubsystem::resolve_principal()` or `Principal::from_user()`
  with real role + permissions.

## End-to-end tests

If your diff touches `crates/{app,kernel,channels,acp,sandbox}/src/`,
add or extend a Rust e2e test in the corresponding `tests/` directory
following `docs/guides/e2e-style.md`:

- **Lane 1**: no LLM. Pure-data scenarios.
- **Lane 2**: scripted LLM via `ScriptedLlmDriver`.
- **Lane 3**: real LLM in `e2e.yml` (CI-only).

If PR-time e2e coverage is genuinely infeasible, state in the PR body
which lane applies and why.

## Outcome evidence (the BE bar)

`cargo test -p <crate>` passing is **not** by itself outcome
verification. Paste:

1. **Test summary lines** verbatim:
   ```
   test result: ok. 83 passed; 0 failed; 3 ignored; 0 measured; 0 filtered out
   ```
2. **Concrete before/after evidence** of the user-visible behavior change.
   For an API change, this is `curl` against the remote backend before
   and after applying the patch (see `docs/guides/debug.md` for the
   remote-backend dev loop). Example: *"before this PR
   `curl /api/sessions` returned `500` with body `{...}`; after this PR
   it returns `200` with body `{...}`."*
3. For lane 1: the `agent-spec lifecycle` summary plus the BDD scenario
   names that passed.

## PR labels

- **Type** (one of): `bug`, `enhancement`, `refactor`, `chore`, `documentation`.
- **Component** (one of for backend work): `core`, `backend`, `ci`.
  Use `core` for kernel / cross-crate concerns; `backend` for HTTP / API /
  service layer; `ci` for CI-only changes that happen to live under
  `crates/**`.

`labeler.yml` auto-labels by file path, but you must still add the type +
component labels explicitly via `--label`.
