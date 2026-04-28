# Boxlite Runtime Staging

`rara-sandbox` wraps boxlite (a microVM library) for hardware-isolated code
execution. Boxlite needs five runtime files to be present at a stable
user-data path before the first VM will start:

- `boxlite-shim`
- `boxlite-guest`
- `mke2fs`
- `debugfs`
- the versioned `libkrunfw` SONAME (e.g. `libkrunfw.5.dylib` on macOS, or
  `libkrunfw.so.5` on Linux). Boxlite's runtime `dlopen`s the versioned
  filename — the unversioned `libkrunfw.dylib`/`libkrunfw.so` is **not**
  required.

`rara setup boxlite` downloads the official prebuilt runtime tarball from
the boxlite GitHub release page and extracts those files into the
platform user-data path. No `cargo build` is required first — staging is
a self-contained download → verify → copy pipeline.

## When to run

- **First-time setup** on a developer machine that will use sandboxed
  code execution.
- **After bumping the boxlite tag** in `crates/rara-sandbox/Cargo.toml` —
  the destination directory is keyed by version, so a new tag means a
  fresh download. `BOXLITE_VERSION` in
  `crates/cmd/src/setup/boxlite.rs` must move lockstep with that tag.

## Usage

```bash
# Download + stage the runtime files into the platform user-data dir.
cargo run -p rara-cli -- setup boxlite

# Dry-run: print the planned URL + destination without touching network or disk.
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

## Mirrored / air-gapped installs

Set the `BOXLITE_RUNTIME_URL` environment variable to point at a mirrored
copy of the upstream tarball. The variable matches upstream `build.rs`'s
own override knob; when set, it takes precedence over the derived
`{base}/{version}/boxlite-runtime-{version}-{target}.tar.gz` URL.

```bash
BOXLITE_RUNTIME_URL=https://mirror.example.com/boxlite-runtime-v0.8.2-darwin-arm64.tar.gz \
  cargo run -p rara-cli -- setup boxlite
```

## Idempotence

Re-running `rara setup boxlite` on an already-staged directory is a
no-op — the `.complete` stamp written at the end of staging is checked
first and reported as "already staged". No HTTP request is made on the
idempotent path.

## Failure modes

- **Tarball missing a required file** → loud error naming the missing
  file. No `.complete` stamp is written, so a re-run will retry.
- **Network failure / 4xx / 5xx** → loud error including the URL.
- **Unsupported platform** (anything other than `darwin-arm64`,
  `linux-x64-gnu`, `linux-arm64-gnu`) → loud error naming the
  `(os, arch)` pair. boxlite has no release artefact for the host.

## Supported platforms

The upstream release pipeline ships tarballs for:

- `darwin-arm64` (macOS, Apple Silicon)
- `linux-x64-gnu`
- `linux-arm64-gnu`

Other targets — including macOS x86_64 — are not currently supported by
upstream and `setup boxlite` errors loudly on them.

## CI

The Linux `clippy` / `test` / `docs` jobs in
`.github/workflows/rust.yml` build with `BOXLITE_DEPS_STUB="1"` to avoid
pulling the full native build chain (meson, ninja, patchelf) onto the
`arc-runner-set` image. Staging itself is exercised in unit tests via a
hermetic in-process HTTP fixture — no real network access from CI.

There is no CI job that downloads the real upstream tarball today. The
self-hosted macOS runner introduced in #1842 was removed in #1916
because its network reachability was too unreliable to gate every PR
on. Real-tarball staging happens only on developer machines until a
stable runner is provisioned — see #1842 for the long-term plan.
