spec: task
name: "issue-1981-e2e-config-path-fix"
inherits: project
tags: ["bug", "ci", "app", "harness"]
---

## Intent

`.github/workflows/e2e.yml` (the lane-3 real-LLM job that PR 1977 codified
as part of the e2e contract) has been red on every `main` push since at
least 2026-04-27. Root cause: `crates/app/src/lib.rs` `start_with_options`
resolves the config-file-sync watch path as `$CWD/config.yaml` only, while
`AppConfig::new` (same file, lines 438-443) resolves it as
`rara_paths::config_file()` (XDG global) merged with `$CWD/config.yaml`
(local override). When the runtime is launched in a directory that has no
local `config.yaml` and the only config lives at the XDG path —
the exact arrangement PR 1948 set up on the ARC runner via
`XDG_CONFIG_HOME=$RUNNER_TEMP/xdg-config` to work around read-only `$HOME`
— `AppConfig::new` succeeds, then `ConfigFileSync::new` panics with
"Failed to initialize config file sync: No such file or directory".

The lie this spec corrects is the comment at lib.rs:566:

```
    // Resolve config file path (same logic as AppConfig::new)
    let config_path = {
        let mut path = std::env::current_dir().unwrap_or_default();
        path.push("config.yaml");
        path
    };
```

The comment claims parity with `AppConfig::new`. The code doesn't honour
it.

**Prior art wall (must read before drafting any alternative).** The
identical fix already exists on the never-merged branch
`issue-1850-live-e2e-in-ci`, commit `4f1e7f8b` ("fix(app): resolve
ConfigFileSync path via XDG fallback (#1850)", authored 2026-04-26). That
commit added an XDG fallback inline at this exact spot, but the PR
appears to have been replaced by a different #1850 commit
(`29ed2051 feat(ci): run live Playwright suite against real backend`)
that did NOT include the lib.rs fix. Issue 1850 is still OPEN. PR 1948's
maintainer assumed #1850's lib.rs fix had landed (the workflow shim only
makes sense alongside that fix); it had not. So `e2e.yml` shipped already
broken — there is no "regression window", main never had the fix.

`gh run list --workflow=e2e.yml --limit 20` shows 18 consecutive
`failure` conclusions back to and including the very first run on
`a89d4f87` (PR 1948's own merge). Every push since has been red.

Reproducer (concrete): from the workspace root, `mv config.yaml /tmp/`,
ensure `~/.config/rara/config.yaml` (or platform-equivalent
`rara_paths::config_file()`) exists with valid contents, then run
`cargo test -p rara-app --test anchor_checkout_e2e -- --ignored`. The
test calls `AppConfig::new()` (succeeds, picks up the XDG config), then
`start_with_options` (panics inside `ConfigFileSync::sync_from_file` —
ENOENT on `$CWD/config.yaml`). The panic message is the one currently
flooding the e2e.yml logs.

Goal alignment: advances `goal.md` Current focus 2026-Q2 bullet 4
("agent harness") on two fronts. First, lane-3 e2e is the only signal
that "rara survives a real LLM end-to-end" — keeping it red defeats
signal 1 ("the process runs for months without intervention") because
production-shape startup is broken. Second, the harness portion (init.sh
agenda check) directly serves "the agent harness this document is part
of" by surfacing red CI on session start, preventing the recurrence
pattern where PR 1977 declared lane 3 working while it was silently red.

Does not cross any "What rara is NOT" line. This is internal stability
infrastructure, not new user-facing surface and not multi-tenancy.

## Decisions

- **Path resolution: shared private helper, not a public-API change.**
  Extract a private `resolve_config_path() -> PathBuf` helper in
  `crates/app/src/lib.rs` that mirrors `AppConfig::new`'s precedence:
  prefer `$CWD/config.yaml` when it exists, else fall back to
  `rara_paths::config_file()`. Both `AppConfig::new` (for the load step)
  and `start_with_options` (for the `ConfigFileSync` watch target) call
  this helper. This is option A from the user's prompt. Option B
  (return `(AppConfig, PathBuf)` from `AppConfig::new`) was rejected:
  changing the signature ripples to every caller (CLI, gateway, tests,
  benchmarks) for a bug whose blast radius is two lines. Two callers,
  one helper, zero public-API churn.

- **Re-land mechanically, do not re-derive.** The body of
  `resolve_config_path` is identical to the inline block on
  `4f1e7f8b:crates/app/src/lib.rs` lines 357-365. The implementer should
  cite that commit in the body of the new commit message ("re-lands the
  fix originally authored on the abandoned `issue-1850-live-e2e-in-ci`
  branch"). This makes the regression-decision audit trail explicit so
  the next #1850-shaped accident (a feature PR ships a workflow change
  that depends on a code change still in someone's draft) is harder to
  repeat.

- **Harness counterpart: extend `init.sh` Agenda section.** Add one
  warn-only check that calls
  `gh run list --workflow=e2e.yml --branch main --limit 1 --json conclusion --jq '.[0].conclusion'`
  and prints `warn "e2e.yml on main is <conclusion> — see <run-url>"`
  when the result is `failure`, `cancelled`, or `timed_out`. Warn-only
  (not fail) because (a) e2e.yml uses real LLM tokens and may legitimately
  be down for cost reasons; (b) `init.sh` already treats third-party
  reachability as warn-only and this is the same shape. The check is
  guarded behind the existing `gh auth status` check so unauthenticated
  sessions do not break.

- **Single PR, two changes, one root cause.** `lib.rs` fix + `init.sh`
  agenda check ship together. They are the bidirectional fix to the same
  failure mode (silently red e2e.yml). Splitting into two issues would
  decouple "the fix" from "the visibility that catches the next
  fix-that-didn't-land", which is what produced this bug in the first
  place. Per `specs/README.md` the lane is determined by whether at least
  one `Test:` selector binds to a real test — `anchor_checkout_roundtrip`
  binds, so this is lane 1. The `init.sh` change is a non-bound but
  small (~6 lines) co-traveller; it does not get its own `Test:` selector.

## Boundaries

### Allowed Changes

- `crates/app/src/lib.rs` — extract `resolve_config_path()` helper, use
  it in both `AppConfig::new` and `start_with_options`. The helper is
  private (`fn`, not `pub fn`).
- `crates/app/tests/anchor_checkout_e2e.rs` — adjust the test setup so
  that the BDD scenario below runs (point `$CWD` at a directory with no
  local `config.yaml` while a valid config exists at
  `rara_paths::config_file()`). This may require a small `tempfile`-based
  helper. Do not duplicate the test; extend the existing one or factor a
  shared setup if needed.
- `init.sh` — append one warn-only gh-query check to the existing
  `Agenda` section. Total addition expected to be under 15 lines.
- New unit test in `crates/app/src/lib.rs` (or `tests/`) that asserts
  `resolve_config_path()` returns the local path when both exist and the
  XDG path when only XDG exists. Pure, no I/O beyond a `tempdir`.
- **/crates/app/src/lib.rs
- **/crates/app/tests/anchor_checkout_e2e.rs
- **/init.sh
- **/specs/issue-1981-e2e-config-path-fix.spec.md

### Forbidden

- Do NOT change `AppConfig::new`'s public signature. The fix is internal.
- Do NOT touch `.github/workflows/e2e.yml`. The XDG_CONFIG_HOME shim
  added by PR 1948 is correct and stays.
- Do NOT touch `crates/app/src/config_sync.rs`. `ConfigFileSync` already
  takes a `PathBuf`; the bug is at the call site, not in the type.
- Do NOT touch `rara_paths`. The XDG resolver is correct; the bug is in
  the consumer.
- Do NOT touch other crates' config loading paths. Only `crates/app/src/lib.rs`
  has this duplicated path-resolution logic; do not pre-emptively refactor
  similar-looking code elsewhere.
- Do NOT make the `init.sh` check fail-fatal. Warn-only. The check is
  there to surface, not to block.
- Do NOT cherry-pick `4f1e7f8b` directly — it predates several
  refactors and may have merge conflicts. Re-author the helper from the
  current `main` and reference the prior commit in the message.

## Acceptance Criteria

```gherkin
Feature: ConfigFileSync resolves config.yaml via the same precedence as AppConfig::new

  Scenario: start_with_options succeeds when only the XDG config exists
    Given a temp dir is set as CWD with no local config.yaml
    And a valid config.yaml exists at rara_paths::config_file()
    When the test calls AppConfig::new() then start_with_options()
    Then start_with_options returns Ok and the app handle becomes ready
    And ConfigFileSync watches the XDG path, not a non-existent CWD path

    Level: integration
    Test Double: real filesystem (tempdir + XDG override env)
    Targets: crates/app/src/lib.rs::start_with_options, crates/app/src/lib.rs::resolve_config_path
    Test: tests/anchor_checkout_e2e.rs::anchor_checkout_roundtrip
```

```gherkin
Feature: resolve_config_path mirrors AppConfig::new precedence

  Scenario: local CWD config wins when both exist
    Given a CWD with a local config.yaml
    And rara_paths::config_file() also exists
    When resolve_config_path is called
    Then it returns the CWD path

    Level: unit
    Test Double: tempdir for both CWD and XDG
    Targets: crates/app/src/lib.rs::resolve_config_path
    Test: src/lib.rs::tests::resolve_config_path_prefers_local

  Scenario: falls back to XDG when CWD has no local override
    Given a CWD with no local config.yaml
    And rara_paths::config_file() exists
    When resolve_config_path is called
    Then it returns rara_paths::config_file()

    Level: unit
    Test Double: tempdir for XDG, empty CWD
    Targets: crates/app/src/lib.rs::resolve_config_path
    Test: src/lib.rs::tests::resolve_config_path_falls_back_to_xdg

  Scenario: ConfigFileSync surfaces a clear error when neither path exists
    Given a CWD with no local config.yaml
    And no file at rara_paths::config_file()
    When start_with_options runs and reaches ConfigFileSync::new
    Then it returns Err whose chain mentions the resolved path
    And the error message names the file actually attempted (the XDG path),
        so the operator can tell which file is missing without reading source

    Level: unit
    Test Double: pure formatter, no I/O
    Targets: crates/app/src/lib.rs::config_file_sync_failure_message, crates/app/src/lib.rs::start_with_options
    Test: src/lib.rs::tests::config_file_sync_failure_message_names_resolved_path
```

## Constraints

- Style anchors: helper signature is `fn resolve_config_path() -> PathBuf`,
  no `Result`, no error handling — matches the panicky behaviour of
  `AppConfig::new`'s use of `unwrap_or_else(|_| PathBuf::from("."))` for
  `current_dir`. Returning a non-existent path is the caller's problem
  (the existing `is_file()` check on `local` keeps this safe).
- The implementer must verify locally before pushing by running:
  `mv config.yaml /tmp/restore-config.yaml.$$ && \
   cargo test -p rara-app --test anchor_checkout_e2e -- --ignored \
       resolve_config_path && \
   mv /tmp/restore-config.yaml.$$ config.yaml`
  i.e. confirm the unit-test scenarios pass without a workspace
  config.yaml present. The real-LLM e2e on a key-bearing CI run is the
  ultimate signal, but the unit tests run on every PR.
- The `init.sh` check must not slow `init.sh` materially — `gh run list`
  with `--limit 1` and `--json` is sub-second; if it ever exceeds 5s
  in practice, wrap it in `timeout 5` and warn on timeout.

## Out of Scope

- Generalising config-path resolution to a top-level `rara_paths`
  utility. There are exactly two callers; a private helper is the right
  shape.
- Backporting the fix to any release branch. There are no release branches.
- Adding a fail-fatal CI gate to detect "PR adds workflow that references
  unmerged code". That is a different harness improvement worth a
  separate issue if anyone wants to pursue it.
- Investigating why `4f1e7f8b` never reached `main` (squash-vs-merge
  policy, force-push, branch-replacement). Forensics is interesting but
  not load-bearing for the fix.
