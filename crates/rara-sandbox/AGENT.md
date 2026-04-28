# rara-sandbox — Agent Guidelines

## Purpose

Hardware-isolated code execution for rara tools, wrapping the
[boxlite](https://github.com/boxlite-ai/boxlite) microVM runtime behind a
small concrete API.

## Architecture

- `src/lib.rs` — public re-exports only.
- `src/config.rs` — `SandboxConfig` (creation parameters) and `ExecRequest`
  (one-shot command description). Both use `bon::Builder`.
- `src/sandbox.rs` — `Sandbox` handle + `ExecOutcome`. Thin adapter over
  `boxlite::BoxliteRuntime` + `LiteBox`.
- `src/error.rs` — `SandboxError` (snafu) + `Result` alias. All boxlite
  failures funnel through `SandboxError::Boxlite { source }`.

Public surface (intentionally minimal, see #1697/#1698):

- `Sandbox::create(SandboxConfig) -> Result<Sandbox>`
- `Sandbox::exec(ExecRequest) -> Result<ExecOutcome>` where
  `ExecOutcome::stdout: boxlite::ExecStdout` is a
  `futures::Stream<Item = String>`
- `Sandbox::destroy(self) -> Result<()>`
- `SandboxConfig` exposes `volumes: Vec<VolumeMount>`,
  `network: NetworkPolicy`, and `working_dir: Option<String>` — all forwarded
  to boxlite (#1937). `ExecRequest::working_dir` further allows per-exec
  overrides via `boxlite::BoxCommand::working_dir`.
- `VolumeMount` and `NetworkPolicy` are wrapper types over
  `boxlite::runtime::options::VolumeSpec` / `boxlite::NetworkSpec`. We do not
  re-export the boxlite types directly — same rule as the rest of this
  crate's public surface, so the kernel never imports `boxlite::*`.

## Critical Invariants

- **No `SandboxBackend` trait.** Issue #1697 was closed as YAGNI — concrete
  `Sandbox` only. Adding a trait now would be speculative abstraction; it
  can be extracted later if a second backend ever lands.
- **No hardcoded rootfs image / paths.** The image reference is a required
  `SandboxConfig` field; the application layer reads it from YAML and
  passes it through. Do not add an `impl Default for SandboxConfig`.
  `NetworkPolicy` *does* implement `Default` — the value mirrors
  `boxlite::NetworkSpec::default` (`Enabled { allow_net: [] }`) so a YAML
  config that omits `network` keeps the historical `run_code` behavior.
  This is the only `Default` impl in the crate and exists solely to make
  `#[serde(default)]` on `SandboxConfig::network` legal.
- **No noop impls, no mock backend.** `docs/guides/anti-patterns.md`
  forbids silent `Ok(())` trait impls. If you need to test a caller
  without a real VM, fake it at the caller boundary — not inside this
  crate.
- **`Sandbox::destroy` consumes `self`.** The boxlite box lives on in the
  runtime state until `remove` is called; dropping the handle leaks the
  box. Callers that forget `destroy` will accumulate boxes under the
  configured boxlite home directory.
- **All errors go through `snafu`.** Boxlite errors wrap via
  `.context(BoxliteSnafu)?`. Do not introduce `thiserror` or manual
  `impl Error`.
- **`Sandbox` is a single-owner handle.** It inherits whatever
  auto-traits boxlite's `LiteBox` provides — we do not add `Send`/`Sync`
  bounds of our own. Callers that need to share a sandbox across async
  tasks must wrap it in `Arc<tokio::Mutex<Sandbox>>` (or equivalent); do
  not assume `Sync`. If boxlite tightens or loosens those bounds in a
  future release, this crate's surface follows along automatically.

## What NOT To Do

- Do NOT bump boxlite to crates.io — **why:** upstream publish is broken
  as of v0.8.2 (see boxlite CLAUDE.md). Stay on the git tag dependency
  until upstream fixes their publishing pipeline.
- Do NOT add a `Default` impl to `SandboxConfig` — **why:** hardcoded
  defaults in Rust bypass the YAML-config discipline; agents will end up
  silently running against `alpine:latest` from the wrong registry.
- Do NOT re-export every boxlite type — **why:** the whole point of this
  crate is to keep the Tool subsystem independent of boxlite's API churn.
  If a caller needs a boxlite type that isn't re-exported, add a
  purpose-specific wrapper instead of widening the surface.
- Do NOT enable the integration test in CI — **why:** it requires the
  runtime files staging from #1699 and a warm OCI image cache; failing in
  CI would block every unrelated PR.
- Do NOT call `boxlite::init_logging_for` from inside this crate —
  **why:** tracing init is an application-layer concern; library crates
  that install global subscribers fight the host's `tracing` setup.
- Do NOT extend `BOXLITE_DEPS_STUB="1"` to the macOS CI job —
  **why:** the stub is scoped to the Linux `clippy` / `test` / `docs`
  jobs in `.github/workflows/rust.yml` because the `arc-runner-set` image
  lacks meson/ninja/patchelf. The `sandbox-macos` job intentionally
  builds boxlite for real so link-time / FFI / `build.rs` regressions in
  `bubblewrap-sys` and `libkrun-sys` are caught on every PR (#1842). If
  that job starts failing, fix the underlying build issue — do not
  re-add the stub on macOS.

## Network policy fusion

A single [`Sandbox`] carries a single [`NetworkPolicy`]. Multiple rara tools
(`bash`, `run_code`, …) share one per-session VM via
`crates/app/src/sandbox.rs::sandbox_for_session`, so the VM's policy must be
fixed at creation time and cannot vary per call without leaking a less
restrictive setting from the first caller forward.

The fusion rule lives at `crates/app/src/sandbox.rs::fused_network_policy`:

- if **every** sandbox-using tool wants `Disabled`, the result is `Disabled`;
- otherwise the result is `Enabled` with the union of allow-lists. An empty
  allow-list under `Enabled` means full outbound (boxlite's own default), so
  a single full-net contributor (today `run_code`) dominates the union.

When you add a new sandbox-using tool, extend `fused_network_policy` so the
union accounts for the tool's policy. Do **not** add a per-call
`NetworkPolicy` argument back to `sandbox_for_session` — that's the exact
shape that motivated this section (PR #1946 review).

## Dependencies

**Upstream (crates this crate depends on):**

- `boxlite` — git dep at tag `v0.8.2`. Pulls four submodules transitively
  (`bubblewrap`, `e2fsprogs`, `libkrun`, `libkrunfw`). Fresh `cargo fetch`
  may be slow; this is normal.
- `bon`, `futures`, `serde`, `snafu`, `tokio`, `tracing` — standard
  workspace deps.

**Downstream (crates that depend on this one):**

- `rara-app` — `crates/app/src/tools/run_code.rs` exposes the `run_code`
  agent-callable tool; sandboxes are stored per-session in a `DashMap`
  shared with `SandboxCleanupHook`. The hook destroys the VM via the
  kernel's `LifecycleHook::on_session_end` (added in #1700) so each
  session pays the boxlite cold-start cost at most once.

## Boxlite Footguns (from the v0.8.2 spike)

These are the things that bit the spike author and will bite the next
person if they aren't written down.

1. **crates.io publish broken upstream.** Cargo deps MUST use the git
   tag form:
   ```toml
   boxlite = { git = "https://github.com/boxlite-ai/boxlite", tag = "v0.8.2" }
   ```
   Do not retry `boxlite = "0.8.2"` — it will look like it works until
   link time.

2. **Submodules are pulled transitively.** boxlite's build brings in
   `bubblewrap`, `e2fsprogs`, `libkrun`, and `libkrunfw`. If your fresh
   clone fails to build, check that `cargo` actually finished fetching
   the submodules (`~/.cargo/git/checkouts/boxlite-*/` should have all
   four under `deps/` or `src/`).

3. **Runtime files need staging.** boxlite expects the following files
   to be present at its runtime directory before the first box will
   start:
   - `boxlite-shim`
   - `boxlite-guest`
   - `mke2fs`
   - `debugfs`
   - the versioned `libkrunfw` SONAME — `libkrunfw.5.dylib` on macOS or
     `libkrunfw.so.5` on Linux for boxlite v0.8.2. Boxlite's runtime
     `dlopen`s the versioned filename embedded in the binary; an
     unversioned `libkrunfw.dylib`/`libkrunfw.so` symlink is **not**
     required.

   Staging is automated via `rara setup boxlite` (see
   `docs/guides/boxlite-runtime.md`). It downloads the upstream prebuilt
   runtime tarball directly — no `cargo build -p rara-sandbox` is
   required first. The destination is:
   - macOS: `~/Library/Application Support/boxlite/runtimes/v0.8.2/`
   - Linux: `$XDG_DATA_HOME/boxlite/runtimes/v0.8.2/`
     (fallback `~/.local/share/boxlite/runtimes/v0.8.2/`)

   The version segment must match the `tag = "v…"` in this crate's
   `Cargo.toml`; `BOXLITE_VERSION` in `crates/cmd/src/setup/boxlite.rs`
   is enforced against that tag by the `version_matches_sandbox_dep`
   test.
