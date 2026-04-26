// Copyright 2025 Rararulab
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Stage boxlite's runtime artifacts into a stable user-data directory so
//! `rara-sandbox` can find them at runtime without each user manually
//! copying files out of cargo's `target/` tree.
//!
//! See `docs/guides/boxlite-runtime.md` for the user-facing flow.

use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

use snafu::{ResultExt, Whatever};
use tracing::instrument;

use super::prompt;

/// Boxlite version pinned by `crates/rara-sandbox/Cargo.toml`. Bump
/// lockstep with the `tag = "vX.Y.Z"` git dependency there — boxlite stages
/// per-version, so a stale value silently writes to the wrong directory.
///
/// This is a mechanism constant (matches the dependency, not user
/// preference), not a YAML knob.
const BOXLITE_VERSION: &str = "v0.8.2";

/// Files that boxlite's runtime expects to find in its staging directory
/// before the first VM will start. Names mirror
/// `crates/rara-sandbox/AGENT.md` footgun #3.
const RUNTIME_FILES: &[&str] = &[
    "boxlite-guest",
    "boxlite-shim",
    "mke2fs",
    "debugfs",
    #[cfg(target_os = "macos")]
    "libkrunfw.dylib",
    #[cfg(target_os = "linux")]
    "libkrunfw.so",
];

/// Files that should be marked executable on unix.
const EXECUTABLE_FILES: &[&str] = &["boxlite-guest", "boxlite-shim", "mke2fs", "debugfs"];

/// Stamp file boxlite's own embedded-runtime extractor checks to short-
/// circuit re-extraction. We write the same stamp so eager staging is a
/// drop-in replacement for the lazy first-call path.
const COMPLETE_STAMP: &str = ".complete";

/// Outcome of a boxlite staging run, surfaced to the caller for logging /
/// CI assertions.
pub enum StageOutcome {
    /// Files were copied into place (or already present and valid).
    Staged {
        /// Destination directory containing the staged runtime.
        dest: PathBuf,
    },
    /// Build artifacts were not found. This is the expected state in CI
    /// when `BOXLITE_DEPS_STUB=1` was used at build time, or when the user
    /// has not yet run `cargo build -p rara-sandbox`.
    NoArtifacts,
    /// `--check` was requested; the discovered source is reported but
    /// nothing was copied.
    CheckOnly {
        /// Build directory that would be the staging source.
        source: Option<PathBuf>,
        /// Destination directory that would receive the files.
        dest:   PathBuf,
    },
}

/// Stage boxlite's runtime artifacts from cargo's build directory into the
/// platform user-data dir.
///
/// `check_only` skips the copy step — useful for CI smoke tests that want
/// to exercise the code path without depending on a real boxlite build.
#[instrument(skip_all, fields(check_only))]
pub async fn run_boxlite_setup(check_only: bool) -> Result<StageOutcome, Whatever> {
    prompt::print_step("Boxlite Runtime Staging");

    let dest = staged_runtime_dir()?;
    println!("  destination: {}", dest.display());

    let source = locate_build_runtime().whatever_context("failed to scan target/ for boxlite")?;

    if check_only {
        match &source {
            Some(src) => prompt::print_ok(&format!("would stage from {}", src.display())),
            None => println!(
                "  no boxlite build artifacts found (run `cargo build -p rara-sandbox` first, or \
                 BOXLITE_DEPS_STUB was set)"
            ),
        }
        return Ok(StageOutcome::CheckOnly { source, dest });
    }

    let Some(source) = source else {
        prompt::print_err(
            "no boxlite build artifacts found under target/. Either build rara-sandbox without \
             BOXLITE_DEPS_STUB, or skip staging on this platform.",
        );
        return Ok(StageOutcome::NoArtifacts);
    };

    if dest.join(COMPLETE_STAMP).is_file() {
        prompt::print_ok(&format!("already staged at {}", dest.display()));
        return Ok(StageOutcome::Staged { dest });
    }

    stage_runtime(&source, &dest)
        .whatever_context(format!("failed to stage runtime to {}", dest.display()))?;

    prompt::print_ok(&format!(
        "staged {} files at {}",
        RUNTIME_FILES.len(),
        dest.display()
    ));
    Ok(StageOutcome::Staged { dest })
}

/// Resolve the destination staging directory.
///
/// Mirrors boxlite's release-mode embedded-runtime path so that boxlite's
/// own extractor sees our `.complete` stamp and skips redundant work
/// (`crates/rara-sandbox/AGENT.md` footgun #3).
fn staged_runtime_dir() -> Result<PathBuf, Whatever> {
    let Some(base) = dirs::data_local_dir() else {
        snafu::whatever!("could not determine platform data-local directory");
    };
    Ok(base.join("boxlite").join("runtimes").join(BOXLITE_VERSION))
}

/// Find the newest `target/*/build/boxlite-*/out/runtime` directory that
/// holds the expected runtime files.
///
/// The cargo build hash is unstable across feature sets, so we glob and
/// pick the newest match rather than reproducing cargo's hashing.
fn locate_build_runtime() -> std::io::Result<Option<PathBuf>> {
    let target_dir = workspace_target_dir();
    if !target_dir.is_dir() {
        return Ok(None);
    }

    // Profile subdirs to scan. `release` first so production builds win
    // ties when both a debug and release build exist.
    let candidates = ["release", "debug"]
        .iter()
        .map(|profile| target_dir.join(profile).join("build"))
        .filter(|p| p.is_dir())
        .flat_map(|build_dir| collect_boxlite_runtimes(&build_dir))
        .flatten();

    let newest = candidates
        .filter(|p| has_required_files(p))
        .max_by_key(|p| {
            fs::metadata(p)
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH)
        });

    Ok(newest)
}

/// Yield every `boxlite-*/out/runtime` directory inside a
/// `target/<profile>/build/`.
fn collect_boxlite_runtimes(build_dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(build_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with("boxlite-") {
            continue;
        }
        let runtime = entry.path().join("out").join("runtime");
        if runtime.is_dir() {
            out.push(runtime);
        }
    }
    Ok(out)
}

/// True if every required runtime file is present in `dir`.
fn has_required_files(dir: &Path) -> bool {
    RUNTIME_FILES.iter().all(|name| dir.join(name).is_file())
}

/// Walk `target/`. We deliberately ignore `CARGO_TARGET_DIR` —
/// `whisper_install.rs` doesn't read it either, and adding a divergent
/// fallback here would surprise users who have a custom target dir.
fn workspace_target_dir() -> PathBuf {
    // The setup binary is invoked from the workspace root via `just` or
    // `cargo run`; both leave cwd at the workspace root.
    PathBuf::from("target")
}

/// Copy required files from `source` to `dest`, set unix permissions to
/// match boxlite's expectations, then write a `.complete` stamp.
///
/// Writing the stamp last preserves the "atomic enough" guarantee boxlite
/// itself relies on — partial copies leave no stamp, so a re-run will
/// retry instead of silently using a half-staged dir.
fn stage_runtime(source: &Path, dest: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dest)?;

    for name in RUNTIME_FILES {
        let src = source.join(name);
        let dst = dest.join(name);
        // Remove first to avoid `text file busy` if a previous boxlite
        // process still has it open via mmap.
        if dst.exists() {
            fs::remove_file(&dst)?;
        }
        fs::copy(&src, &dst)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = if EXECUTABLE_FILES.contains(name) {
                0o755
            } else {
                0o644
            };
            fs::set_permissions(&dst, fs::Permissions::from_mode(mode))?;
        }
    }

    fs::write(dest.join(COMPLETE_STAMP), BOXLITE_VERSION)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs::File;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn version_matches_sandbox_dep() {
        // Sanity: every staged dir is keyed by this version. If the
        // sandbox crate bumps boxlite, this constant must move with it.
        let cargo_toml = include_str!("../../../rara-sandbox/Cargo.toml");
        assert!(
            cargo_toml.contains(&format!("tag = \"{BOXLITE_VERSION}\"")),
            "BOXLITE_VERSION must match the git tag pinned in rara-sandbox/Cargo.toml"
        );
    }

    #[test]
    fn has_required_files_detects_missing() {
        let dir = tempdir().unwrap();
        assert!(!has_required_files(dir.path()));
        for name in RUNTIME_FILES {
            File::create(dir.path().join(name)).unwrap();
        }
        assert!(has_required_files(dir.path()));
    }
}
