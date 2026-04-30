//! Build script for `tape-codec-zig`.
//!
//! Invokes `zig build-lib -O ReleaseSafe` on `src/codec.zig` to produce a
//! static archive, then emits `cargo:rustc-link-{search,lib}` so the
//! linked Rust crate picks it up. Modeled on `TigerBeetle`'s Rust client
//! `build.rs` pattern, simplified — no `zig build` orchestration, no
//! `zigc` crate dependency, just a direct `Command::new("zig")`.
//!
//! Skipped entirely when the `zig-codec` feature is off; in that case the
//! crate's Rust API still compiles but every entry point returns
//! `Error::FeatureDisabled` instead of dispatching to the static lib.

use std::{env, path::PathBuf, process::Command};

fn main() {
    // Without the feature there is nothing to build — the Rust side has
    // no extern declarations to satisfy.
    if env::var_os("CARGO_FEATURE_ZIG_CODEC").is_none() {
        return;
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let src = manifest_dir.join("src").join("codec.zig");

    // Cargo invalidates this build when any .zig file under src/ changes.
    println!("cargo:rerun-if-changed=src");
    for entry in std::fs::read_dir(manifest_dir.join("src")).expect("read src dir") {
        let entry = entry.expect("read dir entry");
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "zig") {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=.zig-version");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_ZIG_CODEC");

    // Pin the Zig toolchain version. `.zig-version` is the source of truth;
    // `std.json.Stringify.valueAlloc` and friends are 0.16-only spellings,
    // so a 0.15 or 0.17 toolchain produces cryptic Zig compile errors
    // rather than a clean "wrong toolchain" message. Fail fast instead.
    let expected_version = include_str!("./.zig-version").trim();
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
         crates/tape-codec-zig/.zig-version. Install via `mise use zig@{expected_version}` or \
         follow docs/guides/zig-toolchain.md"
    );

    // Output naming: zig emits `lib<name>.a` on unix.
    let lib_name = "tape_codec_zig_static";
    let archive = out_dir.join(format!("lib{lib_name}.a"));

    let emit_bin_arg = format!("-femit-bin={}", archive.display());
    let status = Command::new("zig")
        .arg("build-lib")
        .arg("-O")
        .arg("ReleaseSafe")
        .arg("-fno-stack-check")
        .arg("--name")
        .arg(lib_name)
        .arg(&emit_bin_arg)
        .arg(&src)
        .status()
        .expect("failed to spawn `zig` — is Zig 0.16 installed and on PATH?");

    assert!(
        status.success(),
        "zig build-lib failed (exit={status:?}); confirm `zig version` is 0.16.x"
    );

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static={lib_name}");
}
