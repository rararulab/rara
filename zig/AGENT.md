# zig/ — Agent Guidelines

## Purpose

Top-level Zig project. All Zig source code consumed by Rust crates under
`crates/` lives here, declared as artifacts in a single `build.zig`. Today
the only artifact is `tape_codec_zig_static` (issue #2007 PoC, consumed by
`crates/tape-codec-zig/`); future Zig modules add lib targets here, not
ad-hoc per-crate `build.rs` shellouts.

## Architecture

- `build.zig` — declares every Zig artifact. Each Rust wrapper crate has a
  corresponding `b.addLibrary(.{ .linkage = .static, ... })` target.
- `build.zig.zon` — package metadata (name, version, fingerprint,
  `minimum_zig_version = "0.16.0"`, paths). The fingerprint is a
  permanent identity; do not regenerate it on a whim.
- `.zig-version` — toolchain pin (single source of truth). Every Rust
  `build.rs` that invokes `zig build` reads this file and asserts
  `zig version` matches before compiling.
- `src/` — Zig source files, one per logical module. Today:
  `src/tape_codec.zig`.

Cargo handoff (per consuming Rust crate):

1. `crates/<wrapper>/build.rs` runs `zig build --prefix $OUT_DIR -Drelease=true`
   from this directory.
2. `b.installArtifact(lib)` writes `lib<name>.a` into `$OUT_DIR/lib/`.
3. `build.rs` emits `cargo:rustc-link-search=native=$OUT_DIR/lib` and
   `cargo:rustc-link-lib=static=<name>`.

## Critical Invariants

- **`pic = true` on every static-lib `Module`.** Rust on Linux defaults to
  PIE; linking a non-PIC static archive trips
  `relocation R_X86_64_32 cannot be used against local symbol`. macOS
  does not enforce this, so a missing `pic = true` passes locally and
  breaks the Linux smoke job. Set it on the `Module`, not as a CLI flag.
- **`.zig-version` is the toolchain pin.** Do NOT add per-crate
  `.zig-version` files in `crates/`. Drift between pins is a footgun.
- **Cross-compilation is not wired up for the PoC.** `build.rs` does not
  pass `-Dtarget`; `zig build` defaults to host. If a future migration
  needs cross-compilation, plumb `env::var("TARGET")` from `build.rs`
  into a `-Dtarget=<zig triple>` argument and add a Rust→Zig triple
  mapping. Doing this without thinking about CI cache keys is a trap.

## How to Add a New Zig Module

1. Add `src/<name>.zig` with `export fn` entry points.
2. Add a new lib target in `build.zig`:
   ```zig
   const new_module = b.createModule(.{
       .root_source_file = b.path("src/<name>.zig"),
       .target = target, .optimize = optimize, .pic = true,
   });
   const new_lib = b.addLibrary(.{
       .name = "<name>_static",
       .root_module = new_module,
       .linkage = .static,
   });
   new_lib.bundle_compiler_rt = true;
   b.installArtifact(new_lib);
   ```
3. Create the Rust wrapper crate under `crates/<name>/` with a `build.rs`
   modeled on `crates/tape-codec-zig/build.rs`. Link the named artifact.

## What NOT To Do

- Do NOT invoke `zig build-lib` directly from a Rust `build.rs` —
  **why:** that bypasses `build.zig`, which means PIC, optimize mode,
  target options, and any future dependencies become CLI-flag accidents
  per crate. PR #2008's first revision did exactly this and broke Linux
  CI; the fix was to move the configuration into `build.zig`.
- Do NOT duplicate Zig setup per Rust wrapper crate — **why:** each
  duplicate is one more place a future Zig version bump or codegen flag
  has to be applied. One `build.zig` for the whole workspace.
- Do NOT add a per-crate `.zig-version` — **why:** the version pin lives
  here.
- Do NOT regenerate the `fingerprint` in `build.zig.zon` — **why:** the
  field is a permanent package identity. Regenerating it for an existing
  package is treated as a hostile fork by the Zig package manager.

## Dependencies

- None currently. Future Zig deps go in `build.zig.zon` `.dependencies`,
  fetched at `zig build` time.
- Toolchain: Zig 0.16.x. Install per `docs/guides/zig-toolchain.md`.
