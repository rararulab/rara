# Boxlite Runtime Staging

`rara-sandbox` wraps boxlite (a microVM library) for hardware-isolated code
execution. Boxlite needs five runtime files (`boxlite-guest`,
`boxlite-shim`, `mke2fs`, `debugfs`, and `libkrunfw.dylib`/`.so`) to be
present at a stable user-data path before the first VM will start.

`rara setup boxlite` copies those files out of cargo's build directory
into that path. Run it once per machine, after the first
`cargo build -p rara-sandbox`.

## When to run

- **First-time setup** on a developer machine that will use sandboxed
  code execution.
- **After bumping the boxlite tag** in `crates/rara-sandbox/Cargo.toml` —
  the destination directory is keyed by version, so a new tag means a new
  empty directory.
- **After `cargo clean`** wipes the build artefacts; re-run
  `cargo build -p rara-sandbox` first, then re-stage.

## Usage

```bash
# Build rara-sandbox so boxlite's build.rs downloads the platform tarball
# into target/<profile>/build/boxlite-<hash>/out/runtime/.
cargo build -p rara-sandbox

# Copy the runtime files into the platform user-data directory.
cargo run -p rara-cli -- setup boxlite

# Dry-run: print where the files would come from / go to, without copying.
cargo run -p rara-cli -- setup boxlite --check
```

## Staging paths

| Platform | Destination |
|----------|-------------|
| macOS    | `~/Library/Application Support/boxlite/runtimes/<version>/` |
| Linux    | `$XDG_DATA_HOME/boxlite/runtimes/<version>/` (fallback `~/.local/share/boxlite/runtimes/<version>/`) |

These match boxlite's own embedded-runtime fallback paths, so the eager
stamp file (`.complete`) lets boxlite's lazy extractor short-circuit on
the first VM boot.

## Idempotence

Re-running `rara setup boxlite` on an already-staged directory is a
no-op — the `.complete` stamp written at the end of staging is checked
first and reported as "already staged".

## CI

CI builds `rara-sandbox` with `BOXLITE_DEPS_STUB="1"` to avoid pulling
the full native build chain (meson, ninja, patchelf) onto every runner.
That means CI never actually has runtime files to stage, so
`rara setup boxlite --check` in CI exercises only the code path; it
prints "no boxlite build artifacts found" and exits cleanly. Removing
the stub is tracked in #1842.
