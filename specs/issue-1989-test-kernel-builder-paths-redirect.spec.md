spec: task
name: "issue-1989-test-kernel-builder-paths-redirect"
inherits: project
tags: []
---

## Intent

The `Rust / Test` job in `.github/workflows/ci.yml` runs on `arc-runner-set`,
where `$HOME=/home/runner` exists but is not writable by the runner user, and
`XDG_CONFIG_HOME` is unset. Under those conditions `dirs::config_dir()`
returns `/home/runner/.config`, and any test that touches
`rara_paths::workspace_dir()` (or `config_dir()` / `data_dir()`) tries to
`mkdir_all` under that read-only tree and panics with EACCES (os error 13).

The test that actually fails today is
`rara_kernel::e2e_contract_lane2_scripted::lane2_scripted_single_turn_records_expected_trace`
(added by PR 1973). It hands a `tempfile::tempdir()` to
`TestKernelBuilder::new(tmp.path())`, but `TestKernelBuilder::build` never
redirects `rara_paths` at the global level — it only forwards
`scheduler_dir`. Once the agent loop runs, `agent::build_system_prompt`
calls `rara_paths::workspace_dir()` (`crates/kernel/src/agent/mod.rs:822`),
which resolves through the platform default and panics.

Reproducer (linux, no XDG vars):
1. `HOME=/home/runner` (writable: no), `XDG_CONFIG_HOME` unset.
2. `cargo test -p rara-kernel --test e2e_contract_lane2_scripted -- lane2_scripted_single_turn_records_expected_trace`.
3. Panic: `failed to create workspace directory /home/runner/.config/rara/workspace: Permission denied (os error 13)`.
4. Same job is red on `main` at commit 12eb8ec6 and on PR 1984 / PR 1985 — fixing this unblocks both.

Prior art reviewed:
- PR 1948 / PR 1951 patched only `e2e.yml`'s "Render rara config" step
  (workflow-side `XDG_CONFIG_HOME` shim). It did not touch `ci.yml` and did
  not touch in-process tests.
- PR 1985 (issue 1981) fixes only `crates/app/src/lib.rs` ConfigFileSync;
  its boundaries forbid touching kernel test infrastructure.
- PR 1850 (abandoned) attempted a similar XDG shim but never merged.
- `crates/channels/tests/web_session_smoke.rs` is the established pattern:
  it calls `rara_paths::set_custom_data_dir(&data)` +
  `rara_paths::set_custom_config_dir(&config)` inside a `std::sync::Once`
  before any kernel construction. `TestKernelBuilder` is missing the
  equivalent.

The fix lands inside `TestKernelBuilder::build` (or its `new`) so every
existing and future kernel-DI e2e test inherits the redirect for free.
This is more durable than a CI-side `XDG_CONFIG_HOME` shim because (a) it
also helps local devs whose real `~/.config/rara` would otherwise collide
with test runs, and (b) it does not depend on each new workflow file
remembering to set the env var.

Goal alignment: signal 4 ("every action is inspectable") — the harness
that records traces must run reliably in CI; a green `Rust / Test` is the
precondition for trusting any other signal. Crosses no `NOT` line; this
is harness hygiene, not feature work.

## Decisions

- Fix lives in `crates/kernel/src/testing.rs` inside `TestKernelBuilder`,
  not in `rara_paths` and not in `.github/workflows/ci.yml`. Test-side fix
  is the smallest blast radius and benefits every future kernel-DI e2e.
- Redirect both `set_custom_config_dir` and `set_custom_data_dir` to
  subdirectories of the builder's existing `tmp_dir` (e.g.
  `tmp_dir/rara_config` and `tmp_dir/rara_data`). Both are needed because
  `data_dir()` is reached via `rara_paths::data_dir()` independently of
  `config_dir()`.
- Wrap the two `set_custom_*` calls in a `std::sync::Once` keyed on the
  binary's first `TestKernelBuilder::build`. The `OnceLock`s in
  `rara_paths` panic on second-set; `Once` makes the first build win and
  later builds in the same test binary become no-ops. The redirect points
  at a `OnceLock<TempDir>` owned by the test binary so the directory
  outlives any individual `TestKernelBuilder`.
- Do NOT change `rara_paths::workspace_dir()` semantics. The runtime
  contract is unchanged; only test wiring moves.
- Do NOT add `XDG_CONFIG_HOME` to `ci.yml`. Test-side fix supersedes.
- If `set_custom_*_dir` panics because some earlier call already
  initialized `config_dir`/`data_dir` in the same process, fail loud with
  the existing panic message — do not silently swallow. Tests that need
  the redirect must run before any code that touches
  `rara_paths::config_dir()`, which `TestKernelBuilder::new` enforces by
  doing the redirect before constructing the kernel.

## Boundaries

### Allowed Changes
- crates/kernel/src/testing.rs
- **/crates/kernel/src/testing.rs
- **/specs/issue-1989-test-kernel-builder-paths-redirect.spec.md

### Forbidden
- crates/paths/**
- crates/app/**
- crates/channels/**
- crates/kernel/src/agent/**
- crates/kernel/src/syscall.rs
- .github/workflows/**
- crates/kernel/tests/e2e_contract_lane1_no_llm.rs
- crates/kernel/tests/e2e_contract_lane2_scripted.rs

## Completion Criteria

Scenario: lane 2 scripted e2e passes on a runner with read-only HOME and no XDG vars
  Test:
    Package: rara-kernel
    Filter: e2e_contract_lane2_scripted::lane2_scripted_single_turn_records_expected_trace
  Given the failing test exercises TestKernelBuilder end-to-end and reaches build_system_prompt which calls rara_paths::workspace_dir()
  When TestKernelBuilder::build redirects rara_paths to a tempdir before kernel construction
  Then the test passes without panicking on "failed to create workspace directory"

Scenario: lane 1 e2e remains green after the redirect
  Test:
    Package: rara-kernel
    Filter: e2e_contract_lane1_no_llm::lane1_no_llm_tape_write_persists_without_agent_turn
  Given lane 1 also goes through TestKernelBuilder
  When the redirect is in place
  Then the lane 1 test still passes and observes the same tempdir-rooted paths

Scenario: redirect is idempotent within one test binary
  Test:
    Package: rara-kernel
    Filter: testing::test_kernel_builder_redirect_is_idempotent
  Given a test binary constructs two TestKernelBuilders in sequence
  When the second build runs after the first has already set the custom paths
  Then the second build does not panic and reuses the same custom config and data dirs

## Out of Scope

- Touching rara_paths public API or workspace_dir resolution rules.
- CI workflow changes (ci.yml / e2e.yml).
- Fixing ConfigFileSync (PR 1985) or boxlite setup (PR 1984) — both
  unblock automatically once this lands.
- Adding XDG shims to other test crates that already have their own
  set_custom_* setup (channels, extensions/backend-admin).
