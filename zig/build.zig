//! Top-level Zig build for rara.
//!
//! Declares every Zig artifact consumed by Rust crates under `crates/`.
//! Currently just `tape_codec_zig_static` (issue #2007 PoC). Future Zig
//! modules add a new lib target here, not a new ad-hoc `build.rs` shelling
//! out to `zig build-lib`.
//!
//! Run standalone:
//!
//!   zig build                              # default optimize, host target
//!   zig build -Doptimize=ReleaseSafe
//!   zig build --prefix /tmp/out            # installs lib<name>.a under <prefix>/lib/
//!
//! Cargo invokes this build via `crates/tape-codec-zig/build.rs` with
//! `--prefix $OUT_DIR`, then links `lib<name>.a` from `$OUT_DIR/lib/`.

const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{
        .preferred_optimize_mode = .ReleaseSafe,
    });

    // The root module owns module-level codegen flags. `pic = true` is the
    // structural fix for Linux PIE: rustc on Linux defaults to producing
    // position-independent executables, and a non-PIC static archive trips
    // `relocation R_X86_64_32 cannot be used against local symbol`. macOS
    // does not enforce this so a CLI-flag fix can pass locally and break
    // CI; declaring it on the module makes it part of the artifact's
    // identity instead of a build invocation accident.
    const codec_module = b.createModule(.{
        .root_source_file = b.path("src/tape_codec.zig"),
        .target = target,
        .optimize = optimize,
        .pic = true,
        // Some targets need __divti3 etc. which live in compiler-rt. Bundle
        // it into the static archive so the Rust linker does not have to
        // discover a separate runtime.
        .stack_check = false,
    });

    const codec_lib = b.addLibrary(.{
        .name = "tape_codec_zig_static",
        .root_module = codec_module,
        .linkage = .static,
    });
    codec_lib.bundle_compiler_rt = true;

    b.installArtifact(codec_lib);
}
