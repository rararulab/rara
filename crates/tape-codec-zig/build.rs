//! Build script for `tape-codec-zig`.
//!
//! Invokes `zig build` against the top-level `zig/` project to produce
//! `libtape_codec_zig_static.a`, then emits the cargo link directives.
//! All Zig codegen settings (PIC, optimize mode, target) live declaratively
//! in `zig/build.zig`; this script just orchestrates the cargo ↔ zig handoff.
//!
//! Skipped entirely when the `zig-codec` feature is off; in that case the
//! crate's Rust API still compiles but every entry point returns
//! `Error::FeatureDisabled` instead of dispatching to the static lib.
//!
//! Cross-compilation is not supported by this proof-of-concept — `zig
//! build` defaults to host. If a future migration needs cross-compilation,
//! plumb `env::var("TARGET")` through to a `-Dtarget=<zig triple>`
//! argument.

use std::{env, path::PathBuf, process::Command};

fn main() {
    // Without the feature there is nothing to build — the Rust side has
    // no extern declarations to satisfy.
    if env::var_os("CARGO_FEATURE_ZIG_CODEC").is_none() {
        return;
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));

    // crates/tape-codec-zig -> workspace root -> zig/
    let zig_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("zig"))
        .expect("locate top-level zig/ directory relative to crate manifest");
    assert!(
        zig_root.join("build.zig").is_file(),
        "expected zig/build.zig at {}; the top-level Zig project is the source of truth for codec \
         builds",
        zig_root.display()
    );

    // Cargo invalidates this build when any tracked Zig input changes.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_ZIG_CODEC");
    println!(
        "cargo:rerun-if-changed={}",
        zig_root.join("build.zig").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        zig_root.join("build.zig.zon").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        zig_root.join(".zig-version").display()
    );
    let zig_src = zig_root.join("src");
    for entry in std::fs::read_dir(&zig_src).expect("read zig/src dir") {
        let entry = entry.expect("read dir entry");
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "zig") {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }

    // Pin the Zig toolchain version. `.zig-version` is the source of truth;
    // `std.json.Stringify.valueAlloc` and friends are 0.16-only spellings,
    // so a 0.15 or 0.17 toolchain produces cryptic Zig compile errors
    // rather than a clean "wrong toolchain" message. Fail fast instead.
    let zig_version_path = zig_root.join(".zig-version");
    let expected_version = std::fs::read_to_string(&zig_version_path)
        .expect("read zig/.zig-version")
        .trim()
        .to_string();
    let version_output = Command::new("zig")
        .arg("version")
        .output()
        .expect("failed to spawn `zig version` — is Zig installed and on PATH?");
    let found_version = String::from_utf8_lossy(&version_output.stdout)
        .trim()
        .to_string();
    assert!(
        found_version == expected_version,
        "Zig version mismatch: expected {expected_version}, found {found_version}. See \
         zig/.zig-version. Install via `mise use zig@{expected_version}` or follow \
         docs/guides/zig-toolchain.md"
    );

    let lib_name = "tape_codec_zig_static";

    // `zig build --prefix <out_dir>` installs `lib<name>.a` to `<out_dir>/lib/`.
    let status = Command::new("zig")
        .current_dir(&zig_root)
        .arg("build")
        .arg("--prefix")
        .arg(&out_dir)
        // Zig 0.16's `standardOptimizeOption` exposes `-Drelease=bool`. With
        // `preferred_optimize_mode = .ReleaseSafe` declared in build.zig,
        // `-Drelease=true` resolves to ReleaseSafe. (Older 0.x used
        // `-Doptimize=ReleaseSafe`; that flag was removed.)
        .arg("-Drelease=true")
        .status()
        .expect("failed to spawn `zig build` — is Zig 0.16 installed and on PATH?");

    assert!(
        status.success(),
        "zig build failed (exit={status:?}); confirm `zig version` is 0.16.x and that `cd zig && \
         zig build` succeeds standalone"
    );

    let lib_dir = out_dir.join("lib");
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=static={lib_name}");
}
