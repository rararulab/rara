spec: task
name: "issue-2097-boxlite-runtime-gc"
inherits: project
tags: []
---

## Intent

`rara-cli setup boxlite` stages the runtime into a version-keyed directory
under `dirs::data_local_dir()/boxlite/runtimes/{BOXLITE_VERSION}` (see
`crates/cmd/src/setup/boxlite.rs::staged_runtime_dir`, line 221). The path
is per-version on purpose — boxlite's own embedded extractor checks the
`.complete` stamp inside the same per-version directory and skips re-
extraction. Bumping `BOXLITE_VERSION` correctly *creates* a fresh sibling
dir.

The gap: the staging pipeline never deletes the *old* sibling
directories. Each version carries libkrunfw + boxlite-shim + boxlite-guest
+ mke2fs + debugfs (tens of MB per version). Across the recent bump
cadence (v0.8.2 → v0.9.1 → v0.9.2 → v0.9.3 → v0.9.4 in the last few weeks
per `gh pr list --search boxlite`), a developer who has updated through
each release accumulates every prior version's runtime under
`~/Library/Application Support/boxlite/runtimes/`, with nothing in the
codebase to ever reclaim them.

Reproducer:
1. `mkdir -p ~/Library/Application\ Support/boxlite/runtimes/v0.0.1` and
   drop `~50 MB of arbitrary bytes` inside (mimicking a stale prior
   version). Repeat for `v0.0.2`. Both directories are siblings of the
   real `v0.9.4` target.
2. Run `cargo run -p rara-cli -- setup boxlite` (or invoke
   `run_boxlite_setup(false)` directly in a test against a tempdir).
3. Observed: staging into `v0.9.4` succeeds; `ls
   ~/Library/Application\ Support/boxlite/runtimes/` still shows
   `v0.0.1/`, `v0.0.2/`, and `v0.9.4/` side-by-side. The two stale dirs
   are never reclaimed.
4. Expected: after a successful stage of `v0.9.4`, the only entry under
   `runtimes/` is `v0.9.4/`. The stale siblings are removed.

Prior art reviewed (mandatory search):
- Issues: `gh issue list --search "boxlite cleanup|runtime|GC"` returns
  PR-tracking issues (#1696/#1697/#1698/#1699/#1844 — initial sandbox +
  staging integration) and #1980/#1984 (self-sufficient `setup boxlite`
  via release tarball). None mention multi-version GC. No prior issue or
  PR proposes or rejects runtime-dir cleanup.
- PRs: `gh pr list --search boxlite` shows the recent bump cadence
  (#2073/#2076/#2077/#2089/#2093 = v0.8.2 → v0.9.1/2/3/4) plus the
  staging introduction (#1844) and the self-sufficient setup (#1984).
  None of them address GC.
- Commits: `git log --grep=boxlite --since=180.days` confirms the same
  set; the most recent staging-area change is 793395bc (v0.9.4 bump).
  No prior decision was made to *retain* old runtime dirs — the absence
  of GC is an oversight, not a design decision being reversed.
- `rg "runtimes"` inside `crates/cmd` and `crates/rara-sandbox` shows
  the only writer of that directory tree is `staged_runtime_dir` itself.
  There is no other consumer that would care about historical sibling
  dirs (boxlite reads only its own version-keyed dir).

Goal alignment: signal 1 ("the process runs for months without
intervention. Memory does not grow unboundedly, file descriptors do not
leak"). The bug is the disk-leak analogue of an unbounded-growth
violation — every version bump permanently grows the on-disk footprint
of a long-running rara install. Crosses no `NOT` line; this is hygiene
for a single-user local install.

## Decisions

- GC runs at the end of `run_boxlite_setup_with` on **every** successful
  stage, including the idempotent skip path that returns early when
  `.complete` already exists. Rationale: a user who ran
  `rara-cli setup boxlite` *before* the bump (current version staged,
  no cleanup) and then runs it *again* after the bump expects the second
  invocation to also be a janitor. Limiting GC to the download path
  would make `rara-cli setup boxlite` silently useless as a cleanup
  command in exactly the case the user is most likely to invoke it. The
  cost of running GC on the skip path is one `read_dir` of the
  `runtimes/` parent — negligible.
- GC does NOT run in `--check` mode. `--check` is a pure dry-run by
  contract (see existing `check_only_is_pure_dry_run` test). Adding a
  filesystem mutation under `--check` would silently break that
  contract.
- The GC scope is exactly `dest.parent()` — i.e. the `runtimes/`
  directory that holds version-keyed siblings. We do NOT touch anything
  above it (`boxlite/`) and we do NOT recurse below sibling dirs (we
  remove each sibling whole via `remove_dir_all`).
- A "stale sibling" is any direct entry of `dest.parent()` that (a) is a
  directory, (b) has a different name from `dest.file_name()`. We do
  NOT try to validate the sibling's contents look like a boxlite
  runtime, and we do NOT compare its name against any whitelist of
  "known prior versions". Reasoning: any directory at this path is
  rara's to manage — the path is wholly owned by `rara-cli setup
  boxlite`. A non-directory entry (stray file) is left alone; safer to
  ignore than to delete something we did not put there.
- GC failures do NOT fail the setup pipeline. If a sibling dir cannot
  be removed (permissions, file-in-use), log a `tracing::warn!` and
  continue — staging the new version succeeded, which is the user's
  primary outcome. The user can re-run later or clean up manually.
- The GC helper is added as a private function in `setup/boxlite.rs`.
  No new module, no new file — the cleanup is a 20-line addition next
  to the staging pipeline it serves.
- Tests extend the existing hermetic `tempdir()` + `FixtureServer`
  pattern in `mod tests`. We do NOT add an integration test that touches
  `dirs::data_local_dir()`.

## Boundaries

### Allowed Changes
- crates/cmd/src/setup/boxlite.rs
- **/crates/cmd/src/setup/boxlite.rs
- specs/issue-2097-boxlite-runtime-gc.spec.md
- **/specs/issue-2097-boxlite-runtime-gc.spec.md

### Forbidden
- crates/rara-sandbox/**
- crates/cmd/src/setup/whisper_install.rs
- crates/cmd/src/setup/prompt.rs
- crates/cmd/src/setup/mod.rs
- crates/app/**
- crates/paths/**
- .github/workflows/**
- docs/guides/boxlite-runtime.md

## Acceptance Criteria

Scenario: stale sibling runtime dirs are removed after a fresh stage
  Test:
    Package: rara-cli
    Filter: setup::boxlite::tests::stale_sibling_runtime_dirs_removed_on_fresh_stage
  Given the runtimes parent directory holds two stale sibling dirs (v0.0.1, v0.0.2) alongside the target staging dir
  When run_boxlite_setup_with(false, opts) downloads and stages the current version into the target dir
  Then the target dir contains the staged files and a .complete stamp
  And the two stale sibling dirs no longer exist under the runtimes parent

Scenario: stale siblings are also removed on the idempotent skip path
  Test:
    Package: rara-cli
    Filter: setup::boxlite::tests::stale_siblings_removed_on_idempotent_skip
  Given the target dir already contains a valid staged runtime with a .complete stamp
  And a stale sibling dir exists alongside it
  When run_boxlite_setup_with(false, opts) is invoked and short-circuits via the idempotent skip path
  Then the fixture HTTP server receives zero hits
  And the stale sibling dir no longer exists under the runtimes parent
  And the target dir's existing files are unchanged byte-for-byte

Scenario: --check mode performs no GC
  Test:
    Package: rara-cli
    Filter: setup::boxlite::tests::check_mode_does_not_gc_siblings
  Given the runtimes parent holds a stale sibling dir
  When run_boxlite_setup_with(true, opts) runs in check-only mode
  Then the outcome is CheckOnly
  And the stale sibling dir still exists under the runtimes parent

Scenario: stray non-directory entries in the runtimes parent are left alone
  Test:
    Package: rara-cli
    Filter: setup::boxlite::tests::stray_files_in_runtimes_parent_are_preserved
  Given the runtimes parent contains a stray regular file alongside the target staging dir
  When run_boxlite_setup_with(false, opts) stages the current version
  Then the staging succeeds
  And the stray regular file still exists under the runtimes parent

Scenario: GC failure on a sibling does not fail the setup pipeline
  Test:
    Package: rara-cli
    Filter: setup::boxlite::tests::gc_failure_on_sibling_does_not_fail_setup
  Given a stale sibling dir exists alongside the target dir but cannot be removed (e.g. on unix the sibling dir's parent has its write bit cleared)
  When run_boxlite_setup_with(false, opts) stages the current version
  Then the outcome is Staged with the target dir
  And the target dir contains the staged files and a .complete stamp

## Out of Scope

- A `--force` flag, a `setup boxlite --gc` flag, or any new CLI surface.
- App-startup auto-GC (e.g. running cleanup from `rara-app` boot). The
  setup command is the right venue; boot is not.
- Doctor / diagnostic command changes.
- Touching boxlite's own embedded extractor or the `rara-sandbox` crate.
- Changing the `staged_runtime_dir()` path layout.
- Whitelisting / version-history-aware retention (e.g. "keep the last N
  versions"). Single-user local install — one current version is
  enough; a user who downgrades can re-run `setup boxlite`.
- GC for the whisper install path. Out of scope; same pattern can be
  added there in a follow-up if it has the same disk-leak shape.
