# Zig 0.16 Toolchain — Optional, PoC-only

This file documents how to install Zig 0.16 for the issue #2007 codec
PoC. Zig source lives in the top-level `zig/` project; the Rust FFI
wrapper is `crates/tape-codec-zig/`. Zig is **not required** for default
rara development; only contributors who pass `--features zig-codec` to a
kernel build, or who run `cargo test -p tape-codec-zig`, need it.

If `POC_RESULTS.md` lands with a "do not adopt Zig" conclusion, this
file goes away with the rest of the PoC.

## Version

Pinned to **0.16.0** via `zig/.zig-version` (single source of truth —
do not add per-crate `.zig-version` files). Note
this also matches the Zig version transitively required by `zlob`
(workspace pin `=1.3.2`, see top-level `Cargo.toml`), so the toolchain
already exists in the CI image used by `zlob`-touching PRs.

## Install

### macOS

```bash
brew install zig            # currently 0.16.x in homebrew-core
zig version                 # expect 0.16.0 (or 0.16.x)
```

### Linux

```bash
# Option A: official tarball
curl -L https://ziglang.org/download/0.16.0/zig-linux-x86_64-0.16.0.tar.xz \
    | tar -xJ -C "$HOME/.local/share"
ln -sf "$HOME/.local/share/zig-linux-x86_64-0.16.0/zig" "$HOME/.local/bin/zig"

# Option B: mise / asdf
mise use -g zig@0.16.0
```

### Verifying

```bash
zig version
# 0.16.0
```

## Why no `just zig-toolchain-check`

The spec's Decisions section originally proposed a `just
zig-toolchain-check` recipe wired into `init.sh` only when the
`zig-codec` feature is active. Two practical issues prevent that:
`init.sh` does not know which cargo features the next command will
use, and `just` recipes cannot conditionally add themselves to the
listing. The PoC instead leaves Zig out of `init.sh` entirely and
documents the install path here. If the PoC graduates to production,
the right move is to add Zig as a fatal `init.sh` check at the same
time the kernel feature flips to default-on.
