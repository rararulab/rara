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

## Critical Invariants

- **No `SandboxBackend` trait.** Issue #1697 was closed as YAGNI — concrete
  `Sandbox` only. Adding a trait now would be speculative abstraction; it
  can be extracted later if a second backend ever lands.
- **No hardcoded rootfs image / paths.** The image reference is a required
  `SandboxConfig` field; the application layer reads it from YAML and
  passes it through. Do not add an `impl Default for SandboxConfig`.
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
   - `boxlite-guest`
   - `libkrunfw.dylib` (macOS) / `libkrunfw.so` (linux)
   - `mke2fs`
   - `boxlite-shim`
   - `debugfs`

   Staging is automated via `rara setup boxlite` (see
   `docs/guides/boxlite-runtime.md`). The destination is:
   - macOS: `~/Library/Application Support/boxlite/runtimes/v0.8.2/`
   - Linux: `$XDG_DATA_HOME/boxlite/runtimes/v0.8.2/`
     (fallback `~/.local/share/boxlite/runtimes/v0.8.2/`)

   Run `cargo build -p rara-sandbox` once (without `BOXLITE_DEPS_STUB`)
   so boxlite's build.rs downloads the platform tarball, then run
   `rara setup boxlite` to copy the files into place. The version
   segment must match the `tag = "v…"` in this crate's `Cargo.toml`.
