spec: task
name: "issue-1980-setup-boxlite-self-sufficient"
inherits: project
tags: ["cmd", "setup", "sandbox", "boxlite"]
---

## Intent

`./bin/rara setup boxlite` must be self-sufficient end-to-end, the same
way `rara setup whisper` is. Today it is not: on a fresh checkout (no
prior `cargo build`) it errors with "no boxlite build artifacts found
under target/", and even after the documented `cargo build -p
rara-sandbox` the staging still fails because the actual build artifacts
on disk do not match what `RUNTIME_FILES` expects. Setup is supposed to
*do* the work, not narrate prerequisites the user has to satisfy.

Concrete reproducer (verified 2026-04-28 on remote `raratekiAir`,
darwin-arm64, repo at `/Users/rara/code/rararulab/rara`, boxlite tag
`v0.8.2`):

1. After `cargo build -p rara-sandbox` (default mode, no
   `BOXLITE_DEPS_STUB`), inspect what is actually produced:
   `target/{release,debug}/build/boxlite-<hash>/out/runtime/` contains
   exactly `debugfs`, `mke2fs`, `libkrunfw.5.dylib`. It does NOT contain
   `boxlite-guest`, does NOT contain `boxlite-shim`, and does NOT
   contain a bare `libkrunfw.dylib`.
2. Run `./bin/rara setup boxlite`. Output: "no boxlite build artifacts
   found under target/. Either build rara-sandbox without
   BOXLITE_DEPS_STUB, or skip staging on this platform." Exit code
   non-zero (well — `Ok(StageOutcome::NoArtifacts)`, but the user-visible
   message is an error and no staging happened).
3. The published staging directory at
   `~/Library/Application Support/boxlite/runtimes/v0.8.2/` is empty,
   so the next `Sandbox::create` call inside rara-app falls through to
   boxlite's own embedded-runtime extractor — which itself errors
   because the rara binary was not compiled with `embedded-runtime`
   feature. Sandboxed code execution is unreachable on a fresh dev
   machine despite the user having run every documented step.

Why the current code reads target/ at all: PR 1844 (issue 1699) was
written under the belief that boxlite's `build.rs` populates
`OUT_DIR/runtime/` with all five files in default `Source` mode. Reading
the upstream `build.rs` at tag `v0.8.2`
(`~/.cargo/git/checkouts/boxlite-0764168f2430805e/da71624/src/boxlite/build.rs`)
shows three modes selected by `BOXLITE_DEPS_STUB`:

- unset → `Source`: builds native -sys crates from source, calls
  `bundle_boxlite_deps` which copies only library files via
  `is_library_file` filter and skips symlinks (lines 80-85). Result on
  darwin-arm64: `mke2fs`, `debugfs`, `libkrunfw.5.dylib` — no
  `boxlite-guest`, no `boxlite-shim`, no unversioned `libkrunfw.dylib`.
  The shim/guest are only collected when the `embedded-runtime` cargo
  feature is on; rara-sandbox does not enable it.
- `1` → `Stub`: skips everything; CI clippy mode.
- `2` → `Prebuilt`: downloads the official
  `boxlite-runtime-v{version}-{target}.tar.gz` from GitHub Releases,
  extracts it, and calls `create_library_symlinks` to add the
  unversioned `libkrunfw.dylib` symlink alongside `libkrunfw.5.dylib`.

I confirmed by `curl`+`tar -tzf` that
`https://github.com/boxlite-ai/boxlite/releases/download/v0.8.2/boxlite-runtime-v0.8.2-darwin-arm64.tar.gz`
contains exactly `boxlite-runtime/{boxlite-shim,boxlite-guest,mke2fs,debugfs,libkrunfw.5.dylib}`.
Equivalent tarballs ship for `linux-x64-gnu` and `linux-arm64-gnu`. This
is the artifact the upstream maintainers ship as "the runtime files for
downstream consumers", and it is the artifact `setup boxlite` should
hand to the user.

The cleanest fix is therefore: `setup boxlite` downloads that tarball
itself, the same way `setup whisper` downloads the whisper.cpp model
file. No `cargo build` dependency, no scanning of `target/`, no
`BOXLITE_DEPS_STUB=2` toggle the user has to know about. The download
+ extract + verify path is the entire setup, which is what makes it
self-sufficient.

Goal alignment: signal 1 ("the process runs for months without
intervention") — bootstrap correctness is upstream of every long-run
property; if a fresh checkout cannot reach a working sandbox, every
year-out goal is gated on a manual workaround. Signal 4 ("every action
is inspectable") indirectly: a working sandbox is what lets `run_code`
produce the inspectable execution traces. Does not cross any "What
rara is NOT" line — this is rara's own bootstrap pipeline, single-user,
local-only, and inspectable on disk via the `.complete` stamp.

Hermes positioning: not applicable. Hermes Agent does not expose a
sandboxed code-execution facility, so there is no upstream behavior to
match or differ from.

Prior art search summary:

- `gh pr list --search "boxlite"` → PR 1840 / 1844 (the original
  staging implementation we are replacing), PR 1881 (real macOS CI
  build, since reverted), PR 1917 (removed sandbox-macos CI job), PR
  1939 / 1946 (sandbox config evolution, unrelated to staging
  mechanism). PR 1844 is the direct prior art for this work; this
  spec supersedes its `target/`-scan strategy with a download strategy.
- `git log --grep boxlite --since=180.days` → no commit since PR 1844
  has touched the `crates/cmd/src/setup/boxlite.rs` staging logic.
  The mechanism has been broken since merge; nobody hit it because
  `--check` (the only path exercised in CI) does not perform the
  copy and exits cleanly even when no artifacts exist.
- `gh issue list --search "boxlite"` → only #1699 (closed by PR 1844)
  and #1702 (sqlx → diesel migration, unrelated). No reverted decision
  to walk back, no in-flight RFC to coordinate with.
- `git log --since=30.days -- crates/cmd/src/setup/boxlite.rs` → empty.
  No recent edits; this is a clean replacement, not a reversion of
  fresh work.

This is **additive in spirit** to PR 1844's contract (the destination
path, the `.complete` stamp, the version-pinning constant, the
idempotence rule all stay) and **replaces** PR 1844's source-discovery
strategy (scan `target/` → download from upstream releases). No
prior decision is being undone; the prior decision was incomplete,
and we are completing it.

## Decisions

1. **Source of truth: download the prebuilt tarball.** `setup boxlite`
   downloads
   `https://github.com/boxlite-ai/boxlite/releases/download/{BOXLITE_VERSION}/boxlite-runtime-{BOXLITE_VERSION}-{target}.tar.gz`
   where `{target}` is one of `darwin-arm64`, `linux-x64-gnu`,
   `linux-arm64-gnu` per the host's `CARGO_CFG_TARGET_OS` /
   `CARGO_CFG_TARGET_ARCH` (resolved at runtime via `std::env::consts`,
   not cargo cfg, since this binary already ran). On any other host,
   exit with a clear "boxlite unsupported on this platform" message —
   no silent skip, per the project's "no noop fallback" rule.
2. **`RUNTIME_FILES` matches what the tarball actually ships** —
   `["boxlite-shim", "boxlite-guest", "mke2fs", "debugfs",
   "libkrunfw.5.dylib"]` on macOS and the equivalent
   `libkrunfw.so.<v>` SONAME on Linux. No bare `libkrunfw.dylib` /
   `libkrunfw.so` is required: boxlite's runtime `dlopen`s the
   versioned SONAME (build.rs comment lines 81-85: "runtime linker
   uses the full versioned name embedded in the binary"). The
   versioned filename is part of the contract; the fix to the constant
   is therefore a correctness fix, not an arbitrary rename.
3. **Discover the libkrunfw filename from the tarball, do not hard-code
   the version**. Boxlite v0.8.2 ships `libkrunfw.5.dylib` today;
   bumping `BOXLITE_VERSION` later may bump the SONAME. After
   extraction, the staging step verifies presence of the four named
   binaries (`boxlite-shim`, `boxlite-guest`, `mke2fs`, `debugfs`) plus
   "exactly one `libkrunfw.<digits>.dylib`" on macOS and "exactly one
   `libkrunfw.so.<digits>(.<digits>)*`" on Linux. Failing this
   invariant is a hard error with a precise message naming the
   tarball URL — never silent.
4. **Pipeline shape mirrors `ensure_whisper`**: detect → download →
   verify → report. Each step a named function in the same file. The
   `--check` flag prints what would be downloaded (URL, expected
   filenames, destination path) without touching the network or the
   filesystem; current `--check` semantics survive but its meaning
   shifts from "is target/ populated" to "is staging complete".
5. **Idempotence preserved**. The `.complete` stamp check at the start
   stays; if the destination already has a valid stamp and the
   required files, `setup boxlite` reports "already staged" and exits
   cleanly without re-downloading. This is byte-for-byte equivalent
   to PR 1844's behavior on the happy path.
6. **Download lives in the same file**. We reuse the `download_file`
   helper pattern from `whisper_install.rs` (reqwest + bytes_stream +
   progress), and add a `tar -xzf` extraction step. The existing
   `flate2` + `tar` crates are already in the workspace
   (`Cargo.lock`); add them as direct deps to `crates/cmd/Cargo.toml`
   if not already there. We do not shell out to `tar` — keep the
   pipeline platform-agnostic and dependency-explicit (matches the
   project rule "errors at application boundaries via `whatever`",
   and shelling out hides failure modes from snafu).
7. **No `cargo build` invocation**. `setup boxlite` does not run
   cargo. The previous "you must run `cargo build -p rara-sandbox`
   first" requirement is removed from `docs/guides/boxlite-runtime.md`.
   Setup downloads what it needs; the user runs cargo when they want
   to build rara, not as a side-quest for staging.
8. **`BOXLITE_VERSION` stays a Rust const**, matched against the git
   tag in `crates/rara-sandbox/Cargo.toml` by the existing
   `version_matches_sandbox_dep` test. This is correct as-is per the
   project's "mechanism vs config" rule — no operator has a real
   reason to override the boxlite version independently of the
   sandbox crate.
9. **Error messages distinguish three states**: (a) destination
   already complete (success), (b) network / extraction failure
   (actionable error with URL), (c) platform unsupported (clear
   exit). The current single "no artifacts" branch goes away because
   it represents a state the new pipeline cannot reach.

## Boundaries

### Allowed Changes

- `crates/cmd/src/setup/boxlite.rs`: replace `locate_build_runtime`,
  `collect_boxlite_runtimes`, `workspace_target_dir`, and the
  `StageOutcome::NoArtifacts` branch with the new
  download → extract → verify pipeline. Keep `staged_runtime_dir`,
  `stage_runtime` (now copying from a temp extraction dir, not from
  `target/`), the `.complete` stamp logic, the `EXECUTABLE_FILES`
  permission rule, and the `version_matches_sandbox_dep` test
  unchanged.
- `crates/cmd/src/setup/boxlite.rs` test module: rename
  `has_required_files_detects_missing` to keep parity with the new
  `RUNTIME_FILES` set, and add a unit test asserting that the
  per-platform tarball URL pattern resolves correctly for
  darwin-arm64, linux-x64-gnu, linux-arm64-gnu.
- `crates/cmd/src/setup/boxlite.rs`: add an integration test that
  uses a hermetic local HTTP fixture (axum or `tiny_http` + a
  pre-built sample tarball checked into `crates/cmd/tests/fixtures/`)
  to exercise the full download → extract → stage path without
  hitting the network. The `BOXLITE_RUNTIME_URL` env var (already a
  hook in upstream's `build.rs`) becomes the override knob the test
  uses to point at the local fixture.
- `crates/cmd/Cargo.toml`: add `flate2` and `tar` as direct deps if
  not already present.
- `docs/guides/boxlite-runtime.md`: update to remove the "run
  `cargo build -p rara-sandbox` first" prerequisite. Document the
  new download flow, the destination path (unchanged), and the
  `BOXLITE_RUNTIME_URL` override for offline / mirrored installs.
- `crates/rara-sandbox/AGENT.md` "footgun #3" paragraph: update the
  required-files list to include the versioned SONAME and remove the
  "build first, then stage" instruction.
- **/crates/cmd/src/setup/boxlite.rs
- **/crates/cmd/Cargo.toml
- **/docs/guides/boxlite-runtime.md
- **/crates/rara-sandbox/AGENT.md
- **/specs/issue-1980-setup-boxlite-self-sufficient.spec.md

### Forbidden

- Do not change `BOXLITE_VERSION` or its location in
  `crates/cmd/src/setup/boxlite.rs`. The constant stays.
- Do not introduce a YAML config knob for the runtime URL or version.
  The override mechanism is the existing `BOXLITE_RUNTIME_URL` env
  var (matches upstream's contract); there is no per-deployment
  reason to bake a URL into `config.yaml`.
- Do not touch `crates/rara-sandbox/Cargo.toml`. The git tag stays at
  `v0.8.2`. This issue is about staging, not bumping boxlite.
- Do not enable the `embedded-runtime` cargo feature on
  `rara-sandbox`. That path doubles the binary size on every build
  and exists to serve a different (Python/Node SDK) shape; rara's
  staging strategy is on-disk under user-data, not embedded in the
  binary.
- Do not add `boxlite` itself or any `boxlite-*` crate as a direct
  dep of `rara-cli` or `crates/cmd`. Setup must not require building
  any boxlite -sys crate.
- Do not shell out to `curl` or `tar`. Download via `reqwest`,
  extract via `flate2 + tar` rust crates. Shelling out hides errors
  from `snafu` and breaks on machines where `tar` lacks
  `--strip-components`.
- Do not delete the `--check` flag. It changes meaning slightly (now
  reports "would download X to Y" instead of "would copy from
  target/X to Y") but the CLI surface and exit codes stay.
- Do not change the destination path
  (`~/Library/Application Support/boxlite/runtimes/<version>/` on
  macOS, XDG fallback on Linux). It is contractual with boxlite's
  own embedded-runtime extractor.

## Acceptance Criteria

Scenario: fresh machine setup completes end-to-end without prior cargo build
  Given a host with no `target/` directory in the repo and an empty
    `~/Library/Application Support/boxlite/runtimes/v0.8.2/`
  And `BOXLITE_RUNTIME_URL` points to a local hermetic HTTP fixture
    that serves a copy of `boxlite-runtime-v0.8.2-darwin-arm64.tar.gz`
  When `run_boxlite_setup(check_only = false)` is invoked
  Then the destination directory contains `boxlite-shim`,
    `boxlite-guest`, `mke2fs`, `debugfs`, `libkrunfw.5.dylib`,
    and `.complete`
  And `boxlite-shim`, `boxlite-guest`, `mke2fs`, `debugfs` are
    mode `0o755` on unix
  And `libkrunfw.5.dylib` is mode `0o644` on unix
  And the function returns `Ok(StageOutcome::Staged { dest })`
  Test:
    Package: rara-cli
    Filter: boxlite::fresh_setup_downloads_and_stages_all_files

Scenario: re-running on an already-staged directory is a no-op
  Given the destination directory already contains all five required
    files plus a valid `.complete` stamp matching `BOXLITE_VERSION`
  When `run_boxlite_setup(check_only = false)` is invoked
  Then no HTTP request is made (assert via fixture-server hit count == 0)
  And the destination is unchanged byte-for-byte
  And the function returns `Ok(StageOutcome::Staged { dest })` with
    the "already staged" log line
  Test:
    Package: rara-cli
    Filter: boxlite::idempotent_skip_when_already_complete

Scenario: --check prints the planned download without touching disk or network
  Given an empty destination directory
  When `run_boxlite_setup(check_only = true)` is invoked
  Then no HTTP request is made
  And the destination directory remains empty (no `.complete` stamp)
  And the function returns `Ok(StageOutcome::CheckOnly { .. })`
  And the printed output names the tarball URL and the destination path
  Test:
    Package: rara-cli
    Filter: boxlite::check_only_is_pure_dry_run

Scenario: tarball missing a required file fails loudly
  Given `BOXLITE_RUNTIME_URL` points to a fixture tarball that omits
    `boxlite-guest`
  When `run_boxlite_setup(check_only = false)` is invoked
  Then the function returns `Err(_)` (`Whatever` context naming
    `boxlite-guest` as the missing file)
  And no `.complete` stamp is written to the destination
  And any partially-extracted files in the destination are removed
    (so a re-run does not see a half-staged dir)
  Test:
    Package: rara-cli
    Filter: boxlite::missing_required_file_in_tarball_errors_cleanly

Scenario: unsupported platform fails loudly with a precise message
  Given a host whose `(target_os, target_arch)` is not one of
    `(macos, aarch64)`, `(linux, x86_64)`, `(linux, aarch64)`
    (simulated by overriding the platform-resolution function in a
    unit test)
  When platform resolution runs at the start of setup
  Then the function returns `Err(_)` (`Whatever` context naming the
    unsupported `(os, arch)` pair)
  And no destination directory is created
  Test:
    Package: rara-cli
    Filter: boxlite::unsupported_platform_errors_cleanly

Scenario: BOXLITE_VERSION still matches the rara-sandbox git tag
  Given `crates/rara-sandbox/Cargo.toml` pins boxlite at tag `vX.Y.Z`
  When the `version_matches_sandbox_dep` test runs
  Then `BOXLITE_VERSION` equals `vX.Y.Z`
  Test:
    Package: rara-cli
    Filter: boxlite::version_matches_sandbox_dep

Scenario: target/ is not consulted at any point in the pipeline
  Given a `target/release/build/boxlite-deadbeef/out/runtime/` populated
    with deliberately-wrong garbage files
  When `run_boxlite_setup(check_only = false)` is invoked
  Then the garbage in `target/` is ignored entirely (no read, no copy)
  And staging proceeds purely from the downloaded tarball
  Test:
    Package: rara-cli
    Filter: boxlite::target_dir_is_never_consulted

## Constraints

- All comments and identifiers in new code must be English (project
  rule).
- Errors at this layer are application-boundary, so `snafu::Whatever`
  + `.whatever_context("...")` is correct (matches `whisper_install.rs`).
  Do not introduce a domain `BoxliteSetupError` enum — there is no
  caller that needs to match on variants.
- The download client is `reqwest` (already a workspace dep used by
  `whisper_install.rs`); the extraction is `flate2::read::GzDecoder`
  + `tar::Archive`. No process-spawning, no `Command::new("tar")`.
- Progress reporting follows `whisper_install.rs`'s every-5%
  pattern. Do not introduce `indicatif` or another progress crate
  for one call site.
- The integration test uses a hermetic in-process HTTP server (a
  `tiny_http` thread or `axum::Router` on a `tokio::spawn` listener
  bound to `127.0.0.1:0`) so CI does not depend on `github.com`
  reachability.
- No new YAML config keys. `BOXLITE_RUNTIME_URL` (env var) is the
  override knob, matching upstream's `build.rs` contract.

## Out of Scope

- Bumping `BOXLITE_VERSION` past `v0.8.2`. Track that separately
  when there is a reason to upgrade.
- Re-enabling the `sandbox-macos` CI job removed in PR 1917. The
  long-term plan from PR 1842 still applies; this issue does not
  change that.
- Replacing rara's "stage to user-data dir" strategy with boxlite's
  `embedded-runtime` cargo feature. The on-disk strategy is
  intentional (binary size, separate update lifecycle for runtime
  vs binary).
- Building the boxlite -sys crates from source on the user's
  machine. The point of this issue is to make that unnecessary.
- Wiring `setup boxlite` into a top-level `setup` orchestrator that
  also runs `setup whisper`. That is a separate UX issue if anyone
  wants it; today both subcommands stand alone.
