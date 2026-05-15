spec: task
name: "issue-2099-boxlite-version-from-cargo-lock"
inherits: project
tags: []
---

## Intent

Bumping the boxlite dependency today requires two manual edits that must
stay in lockstep:

1. `crates/rara-sandbox/Cargo.toml:20` — `tag = "vX.Y.Z"` on the git dep.
2. `crates/cmd/src/setup/boxlite.rs:42` — `const BOXLITE_VERSION: &str = "vX.Y.Z"`.

The only guard against drift is the unit test `version_matches_sandbox_dep`
in `crates/cmd/src/setup/boxlite.rs:738`, which `include_str!`s the sandbox
crate's `Cargo.toml` and string-greps for `tag = "{BOXLITE_VERSION}"`.
That test catches the most obvious mismatch but is fragile (formatting
sensitive, no help if `BOXLITE_VERSION` is forgotten in a future
refactor, and produces an unhelpful failure mode where compilation
succeeds but a unit test fails with a string-diff message).

Reproducer for the silent-failure mode (what happens if the test is
removed, skipped, or weakened):

1. Bump `rara-sandbox/Cargo.toml` to `tag = "v0.9.5"`.
2. Leave `BOXLITE_VERSION` at `"v0.9.4"` in `boxlite.rs`.
3. `cargo build -p rara-cli` succeeds; the sandbox crate links against
   the v0.9.5 boxlite, but `rara setup boxlite` stages the v0.9.4 tarball
   into `~/Library/Application Support/boxlite/runtimes/v0.9.4/`.
4. At runtime, boxlite's release-mode embedded-runtime extractor looks
   for runtimes under the linked-in version (v0.9.5) and finds nothing,
   so it falls back to the (now stale, ABI-skewed) v0.9.4 dir or
   re-extracts. Worst case: SONAME of `libkrunfw` shifted between
   versions, so loading crashes at sandbox start.

Today this is only caught by `version_matches_sandbox_dep`. PR 2098 just
landed runtime-sibling GC, which makes the "wrong directory" outcome
more visible (stale dirs get GC'd) but does not remove the underlying
need for a single source of truth.

The fix: extract `BOXLITE_VERSION` from `Cargo.lock` at build time and
expose it as a `const` from a generated file under `OUT_DIR`. The
sandbox crate's `Cargo.toml` `tag = "..."` becomes the single source of
truth; `BOXLITE_VERSION` derives mechanically from cargo's already-pinned
lockfile entry. The fragile string-grep test becomes either obsolete
(deleted) or reduces to a one-line invariant ("the generated const is
non-empty and starts with `v`").

Cargo.lock shape, verified against the live file:

```
[[package]]
name = "boxlite"
version = "0.9.4"
source = "git+https://github.com/boxlite-ai/boxlite?tag=v0.9.4#45e211fad922a12230025f569f98bf14592c0a08"
```

This matters: the `version` field carries the crate's internal semver
(`0.9.4`, no `v` prefix). The git tag (`v0.9.4`, with `v`) lives only in
the `source` URL's query string (`?tag=v0.9.4`). The current
`BOXLITE_VERSION = "v0.9.4"` path shape includes the `v`, and changing
that shape would orphan every staged dir under
`~/Library/Application Support/boxlite/runtimes/v0.9.4/`. Therefore
build.rs MUST parse the `source` URL fragment, not the `version` field,
and the resulting constant MUST match the existing `vX.Y.Z` shape
exactly.

Precedent for the OUT_DIR-generated-const pattern lives in this same
crate: `crates/cmd/build.rs` already invokes `shadow_rs::ShadowBuilder`
which writes `shadow.rs` to `OUT_DIR`, and `crates/cmd/src/build_info.rs`
consumes it via `shadow_rs::shadow!()`. The mechanism is identical, just
without a third-party builder crate.

Goal alignment: signal 1 ("the process runs for months without
intervention") — sandbox staging silently writing to the wrong directory
is the kind of cross-crate version-skew bug that surfaces months after a
routine dependency bump. Eliminating the two-edit footgun removes a
class of stability bugs at the source. Crosses no `NOT` line — this is
internal mechanism hygiene, not user-visible feature work.

Prior art reviewed:

- PR 2089, 2093, 2073, 2076, 2077 — every recent boxlite bump touched
  both files in lockstep. Dependabot only edits Cargo.toml; the
  follow-up commit that adjusts `BOXLITE_VERSION` is always manual.
  Confirms the two-edit drift surface is real and recurrent.
- PR 2098 (merged minutes before this spec) — added GC of stale boxlite
  runtime sibling dirs. Touches `crates/cmd/src/setup/boxlite.rs` but
  not `BOXLITE_VERSION`; no conflict with the allowed-changes scope
  below.
- Issue 1980 / PR 1984 — established the current self-sufficient setup
  pipeline (tarball download, no `target/` lookup). The
  `BOXLITE_VERSION` constant was deliberately introduced as a mechanism
  constant, not a YAML knob; this spec preserves that decision and only
  changes how the constant is sourced.
- `shadow-rs` precedent — already a `[build-dependencies]` of `rara-cli`
  (`crates/cmd/Cargo.toml:69`). Adding another `OUT_DIR`-writing
  build-script side-effect is the established pattern, not a new one.
- No prior `BOXLITE_VERSION`-from-lockfile attempt found in `gh pr list`
  or `git log --grep`; no prior decision to revert.

## Decisions

- **Source of truth**: `crates/rara-sandbox/Cargo.toml`'s `tag = "..."`
  field is the only place humans (and dependabot) edit a boxlite
  version. `BOXLITE_VERSION` becomes derived state.
- **Mechanism**: extend `crates/cmd/build.rs` to read `Cargo.lock` from
  the workspace root, find the `[[package]] name = "boxlite"` entry,
  extract the `tag=<value>` query parameter from its `source` URL, and
  write `OUT_DIR/boxlite_version.rs` containing
  `pub const BOXLITE_VERSION: &str = "v0.9.4";`.
- **Consumption**: `crates/cmd/src/setup/boxlite.rs` replaces the
  hand-written const with
  `include!(concat!(env!("OUT_DIR"), "/boxlite_version.rs"));` and
  uses the included const exactly where it does today (URL build,
  staging dir path, `.complete` stamp body).
- **Parsing approach**: hand-rolled minimal scan over `Cargo.lock` —
  no new dependency. `Cargo.lock` is a well-known TOML document with
  a stable shape; a 20-line scanner that looks for
  `name = "boxlite"` followed by the next `source = "..."` line and
  extracts the `?tag=...#` substring is simpler than pulling in
  `cargo_metadata` (heavy — runs cargo) or `toml` (workspace doesn't
  use it; build-time-only dep adds compile cost for one parse).
  Rationale: build scripts should be boring. The parse failure modes
  (`Cargo.lock` missing, boxlite entry missing, `source` field
  malformed) are all "this is not a valid rara checkout" — `panic!`
  with a descriptive message is the right response.
- **`v` prefix preservation**: build.rs extracts the literal
  `tag=v0.9.4` value verbatim from the source URL. The leading `v` is
  preserved because that is what is in the URL. The generated const
  matches the current path shape (`vX.Y.Z`) exactly — no migration
  needed, no orphaned staged dirs.
- **`build.rs` rerun-if directives**: emit
  `cargo:rerun-if-changed=../../Cargo.lock` so the build script
  re-runs when (and only when) `Cargo.lock` changes. Stale
  generated files are a worse failure mode than over-eager rebuilds.
- **Replace, don't keep**, the `version_matches_sandbox_dep` test.
  Once the const is derived, the invariant it asserted ("the const
  matches the Cargo.toml tag") is structurally guaranteed by the
  build script. Replace it with a tiny `generated_const_has_v_prefix`
  unit test: `assert!(BOXLITE_VERSION.starts_with('v'))` — cheap, and
  catches the one remaining failure mode (boxlite's tag-naming
  convention silently dropping the `v`).
- **Do NOT** add a build-script dependency (`cargo_metadata`, `toml`).
  Hand-rolled scan is ~20 lines and has no transitive footprint on
  `rara-cli`'s build time, which already pulls in `shadow-rs`.
- **Do NOT** change the staged path shape, the URL shape, or any
  user-visible behavior. This is a pure refactor.

## Boundaries

### Allowed Changes
- crates/cmd/build.rs
- **/crates/cmd/build.rs
- crates/cmd/src/setup/boxlite.rs
- **/crates/cmd/src/setup/boxlite.rs
- specs/issue-2099-boxlite-version-from-cargo-lock.spec.md
- **/specs/issue-2099-boxlite-version-from-cargo-lock.spec.md

### Forbidden
- crates/rara-sandbox/**
- crates/cmd/Cargo.toml
- Cargo.lock
- Cargo.toml
- crates/cmd/src/setup/mod.rs
- crates/cmd/src/setup/prompt.rs
- crates/cmd/src/setup/whisper_install.rs
- .github/workflows/**
- docs/**

## Completion Criteria

Scenario: build script derives BOXLITE_VERSION from Cargo.lock
  Test:
    Package: rara-cli
    Filter: setup::boxlite::tests::generated_const_matches_sandbox_tag
  Given Cargo.lock pins boxlite to tag v0.9.4 via the git source URL
  When the build script runs and crates/cmd/src/setup/boxlite.rs is compiled
  Then the included BOXLITE_VERSION const equals the literal string "v0.9.4"

Scenario: generated const preserves the leading v prefix
  Test:
    Package: rara-cli
    Filter: setup::boxlite::tests::generated_const_has_v_prefix
  Given the boxlite git tag convention uses a leading v (v0.9.4, not 0.9.4)
  When the build script extracts the tag from the source URL
  Then BOXLITE_VERSION starts with 'v' and is non-empty

Scenario: existing staging behavior is unchanged
  Test:
    Package: rara-cli
    Filter: setup::boxlite::tests::fresh_setup_downloads_and_stages_all_files
  Given the refactor only changes how BOXLITE_VERSION is sourced, not its value
  When the existing fresh-setup test runs against the fixture server
  Then it still passes byte-for-byte, with the same destination path and the same .complete stamp contents

Scenario: idempotent re-stage still skips work
  Test:
    Package: rara-cli
    Filter: setup::boxlite::tests::idempotent_skip_when_already_complete
  Given a destination already has a .complete stamp written with the current BOXLITE_VERSION
  When run_boxlite_setup_with runs a second time
  Then it returns Staged without hitting the fixture server, exactly as before the refactor

## Out of Scope

- Bumping boxlite to a new version. This spec lands at the current
  pinned tag (v0.9.4) and is verified against it.
- Touching `crates/rara-sandbox/Cargo.toml` or any sandbox-crate source.
  The whole point is that the sandbox manifest stays the single source
  of truth.
- Replacing `shadow-rs` or refactoring `crates/cmd/build.rs` beyond
  adding the boxlite-version generator.
- Generalizing to other git-tagged deps. boxlite is the only dep with
  this two-edit problem today; speculative generalization violates
  "simplicity first".
- Removing `BOXLITE_VERSION` as a separate compile-time identifier
  (e.g. inlining it into every call site). The const-with-a-name
  remains valuable for `tracing` log lines and grep-ability.
