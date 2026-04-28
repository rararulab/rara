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
//! `rara-sandbox` can find them at runtime — without requiring the user to
//! first run `cargo build -p rara-sandbox`.
//!
//! The pipeline mirrors `whisper_install.rs::ensure_whisper`:
//! detect → download → verify → report. The source of truth is the
//! prebuilt tarball on the boxlite GitHub release page; rara never reads
//! cargo's `target/` directory.
//!
//! See `docs/guides/boxlite-runtime.md` for the user-facing flow.

use std::{
    fs,
    path::{Path, PathBuf},
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

/// Base URL for the upstream boxlite release archive. Appended with
/// `{version}/boxlite-runtime-{version}-{target}.tar.gz`.
const BOXLITE_RELEASE_URL_BASE: &str = "https://github.com/boxlite-ai/boxlite/releases/download";

/// Env var that overrides the full tarball URL. Matches the upstream
/// `build.rs` contract so air-gapped installs and tests can point at a
/// mirror.
const BOXLITE_RUNTIME_URL_ENV: &str = "BOXLITE_RUNTIME_URL";

/// Files the boxlite runtime needs by exact name. The versioned `libkrunfw`
/// SONAME (e.g. `libkrunfw.5.dylib`) is discovered separately because it
/// changes across boxlite versions.
const REQUIRED_NAMED_FILES: &[&str] = &["boxlite-shim", "boxlite-guest", "mke2fs", "debugfs"];

/// Files that should be marked executable on unix.
const EXECUTABLE_FILES: &[&str] = &["boxlite-shim", "boxlite-guest", "mke2fs", "debugfs"];

/// Stamp file boxlite's own embedded-runtime extractor checks to short-
/// circuit re-extraction. We write the same stamp so eager staging is a
/// drop-in replacement for the lazy first-call path.
const COMPLETE_STAMP: &str = ".complete";

/// Outcome of a boxlite staging run, surfaced to the caller for logging /
/// CI assertions.
#[derive(Debug)]
pub enum StageOutcome {
    /// Files were copied into place (or already present and valid).
    Staged {
        /// Destination directory containing the staged runtime.
        dest: PathBuf,
    },
    /// `--check` was requested; the planned URL + destination are
    /// reported but nothing was downloaded or written.
    CheckOnly {
        /// Tarball URL that would be downloaded.
        url:  String,
        /// Destination directory that would receive the files.
        dest: PathBuf,
    },
}

/// Internal options for the staging pipeline. The public entry point fills
/// these from the running platform; tests substitute hermetic values.
struct SetupOptions {
    /// Tarball URL to download. Already overridden by env var if set.
    url:  String,
    /// Destination directory for the staged runtime files.
    dest: PathBuf,
}

/// Stage boxlite's runtime artifacts by downloading the official prebuilt
/// tarball and extracting it into the platform user-data dir.
///
/// `check_only` skips the download + filesystem mutation and only prints
/// what *would* happen — useful for CI smoke tests.
#[instrument(skip_all, fields(check_only))]
pub async fn run_boxlite_setup(check_only: bool) -> Result<StageOutcome, Whatever> {
    prompt::print_step("Boxlite Runtime Staging");

    let target = host_target().whatever_context("boxlite is not supported on this host")?;
    let url = resolve_runtime_url(target);
    let dest = staged_runtime_dir()?;

    let opts = SetupOptions { url, dest };
    run_boxlite_setup_with(check_only, &opts).await
}

/// Core pipeline, parameterised on URL + destination so unit tests can
/// substitute a hermetic HTTP fixture and tempdir without touching
/// `dirs::data_local_dir()` or the network.
async fn run_boxlite_setup_with(
    check_only: bool,
    opts: &SetupOptions,
) -> Result<StageOutcome, Whatever> {
    println!("  destination: {}", opts.dest.display());
    println!("  source:      {}", opts.url);

    if check_only {
        prompt::print_ok("check mode: no download, no copy");
        return Ok(StageOutcome::CheckOnly {
            url:  opts.url.clone(),
            dest: opts.dest.clone(),
        });
    }

    if opts.dest.join(COMPLETE_STAMP).is_file() && has_required_files(&opts.dest) {
        prompt::print_ok(&format!("already staged at {}", opts.dest.display()));
        return Ok(StageOutcome::Staged {
            dest: opts.dest.clone(),
        });
    }

    let tmp = tempfile::tempdir().whatever_context("failed to create temp dir for extraction")?;
    let archive_path = tmp.path().join("boxlite-runtime.tar.gz");

    println!("  downloading {} ...", opts.url);
    download_file(&opts.url, &archive_path).await?;

    let extract_root = tmp.path().join("extract");
    fs::create_dir_all(&extract_root).whatever_context("failed to create extraction directory")?;
    extract_tarball(&archive_path, &extract_root)?;

    let staging_src = locate_extracted_runtime(&extract_root)?;
    verify_extracted_files(&staging_src)?;

    stage_runtime(&staging_src, &opts.dest).whatever_context(format!(
        "failed to stage runtime to {}",
        opts.dest.display()
    ))?;

    prompt::print_ok(&format!("staged at {}", opts.dest.display()));
    Ok(StageOutcome::Staged {
        dest: opts.dest.clone(),
    })
}

// ---------------------------------------------------------------------------
// Platform resolution
// ---------------------------------------------------------------------------

/// `(os, arch)` pair recognised by the upstream release artefact naming.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostTarget {
    DarwinArm64,
    LinuxX64Gnu,
    LinuxArm64Gnu,
}

impl HostTarget {
    /// Tarball suffix as produced by the boxlite release pipeline.
    fn tarball_target(self) -> &'static str {
        match self {
            HostTarget::DarwinArm64 => "darwin-arm64",
            HostTarget::LinuxX64Gnu => "linux-x64-gnu",
            HostTarget::LinuxArm64Gnu => "linux-arm64-gnu",
        }
    }
}

/// Resolve the running host's target. Returns `Err` for unsupported
/// platforms — boxlite simply has no release artefact for them.
fn host_target() -> Result<HostTarget, String> {
    resolve_target(std::env::consts::OS, std::env::consts::ARCH)
}

/// Pure helper for `host_target`, separated so tests can exercise the
/// "unsupported platform" branch without mocking the global env consts.
fn resolve_target(os: &str, arch: &str) -> Result<HostTarget, String> {
    match (os, arch) {
        ("macos", "aarch64") => Ok(HostTarget::DarwinArm64),
        ("linux", "x86_64") => Ok(HostTarget::LinuxX64Gnu),
        ("linux", "aarch64") => Ok(HostTarget::LinuxArm64Gnu),
        _ => Err(format!("unsupported (os, arch) pair: ({os}, {arch})")),
    }
}

/// Build the tarball URL for the given target, honoring the
/// `BOXLITE_RUNTIME_URL` env override (matches upstream's `build.rs`).
fn resolve_runtime_url(target: HostTarget) -> String {
    if let Ok(override_url) = std::env::var(BOXLITE_RUNTIME_URL_ENV) {
        if !override_url.is_empty() {
            return override_url;
        }
    }
    format!(
        "{base}/{version}/boxlite-runtime-{version}-{target}.tar.gz",
        base = BOXLITE_RELEASE_URL_BASE,
        version = BOXLITE_VERSION,
        target = target.tarball_target(),
    )
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

// ---------------------------------------------------------------------------
// Download + extract
// ---------------------------------------------------------------------------

/// Download a file via reqwest with progress display every ~5%.
async fn download_file(url: &str, dest: &Path) -> Result<(), Whatever> {
    use std::io::Write;

    use futures::StreamExt;
    use tokio::io::AsyncWriteExt;

    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .send()
        .await
        .whatever_context("download request failed")?;

    if !resp.status().is_success() {
        snafu::whatever!("download failed: HTTP {} from {url}", resp.status());
    }

    let total = resp.content_length();
    let mut stream = resp.bytes_stream();
    let mut file = tokio::fs::File::create(dest)
        .await
        .whatever_context("failed to create output file")?;

    let mut downloaded: u64 = 0;
    let mut last_pct: u64 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.whatever_context("error reading download stream")?;
        file.write_all(&chunk)
            .await
            .whatever_context("error writing to file")?;
        downloaded += chunk.len() as u64;

        if let Some(total) = total {
            let pct = downloaded * 100 / total;
            if pct >= last_pct + 5 {
                last_pct = pct;
                print!("\r  downloading... {pct}%");
                std::io::stdout().flush().ok();
            }
        }
    }

    file.flush()
        .await
        .whatever_context("failed to flush file")?;

    if total.is_some() {
        println!("\r  downloading... 100%");
    }

    Ok(())
}

/// Extract a `.tar.gz` archive into `dest`. Uses the rust `flate2` + `tar`
/// crates; we do not shell out to `tar` (hides errors from snafu and
/// breaks on machines where `tar` lacks `--strip-components`).
fn extract_tarball(archive: &Path, dest: &Path) -> Result<(), Whatever> {
    let file = fs::File::open(archive)
        .whatever_context(format!("failed to open archive {}", archive.display()))?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(gz);
    tar.unpack(dest)
        .whatever_context(format!("failed to extract archive into {}", dest.display()))?;
    Ok(())
}

/// The upstream tarball wraps everything under a `boxlite-runtime/`
/// directory. Find that directory; if absent (some mirrors flatten the
/// layout), fall back to `dest` itself when it directly holds the files.
fn locate_extracted_runtime(extract_root: &Path) -> Result<PathBuf, Whatever> {
    let nested = extract_root.join("boxlite-runtime");
    if nested.is_dir() {
        return Ok(nested);
    }
    if has_named_files(extract_root) {
        return Ok(extract_root.to_path_buf());
    }
    snafu::whatever!(
        "extracted archive does not contain a `boxlite-runtime/` directory at {}",
        extract_root.display()
    );
}

/// Confirm every required named binary plus exactly one platform-correct
/// versioned `libkrunfw` entry is present in `dir`.
fn verify_extracted_files(dir: &Path) -> Result<(), Whatever> {
    for name in REQUIRED_NAMED_FILES {
        let p = dir.join(name);
        if !p.is_file() {
            snafu::whatever!(
                "tarball is missing required file `{name}` at {}",
                p.display()
            );
        }
    }
    find_libkrunfw(dir)?;
    Ok(())
}

/// Discover the versioned `libkrunfw` filename in `dir`. Returns the
/// just-the-filename string, not the full path. Errors loudly if zero or
/// more than one candidate exists.
fn find_libkrunfw(dir: &Path) -> Result<String, Whatever> {
    let entries = fs::read_dir(dir)
        .whatever_context(format!("failed to read directory {}", dir.display()))?;

    let mut matches: Vec<String> = Vec::new();
    for entry in entries {
        let entry = entry.whatever_context("failed to read directory entry")?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if is_libkrunfw_soname(&name) {
            matches.push(name);
        }
    }

    match matches.len() {
        0 => snafu::whatever!(
            "tarball is missing the versioned libkrunfw library (expected e.g. libkrunfw.5.dylib \
             on macOS or libkrunfw.so.5 on linux) in {}",
            dir.display()
        ),
        1 => Ok(matches.pop().expect("len == 1 just checked")),
        _ => snafu::whatever!(
            "tarball contains multiple libkrunfw candidates {:?}; expected exactly one",
            matches
        ),
    }
}

/// Recognise `libkrunfw.<digits>.dylib` (macOS) or
/// `libkrunfw.so.<digits>(.<digits>)*` (linux). Pure string match; we
/// avoid pulling in `regex` for one call site.
fn is_libkrunfw_soname(name: &str) -> bool {
    if cfg!(target_os = "macos") {
        let Some(rest) = name.strip_prefix("libkrunfw.") else {
            return false;
        };
        let Some(version) = rest.strip_suffix(".dylib") else {
            return false;
        };
        !version.is_empty() && version.chars().all(|c| c.is_ascii_digit())
    } else if cfg!(target_os = "linux") {
        let Some(rest) = name.strip_prefix("libkrunfw.so.") else {
            return false;
        };
        !rest.is_empty()
            && rest
                .split('.')
                .all(|seg| !seg.is_empty() && seg.chars().all(|c| c.is_ascii_digit()))
    } else {
        false
    }
}

/// Cheap "do the four named files exist" probe used as a fallback for
/// flat-layout archives.
fn has_named_files(dir: &Path) -> bool {
    REQUIRED_NAMED_FILES
        .iter()
        .all(|name| dir.join(name).is_file())
}

/// Full "is the destination already a valid staging dir" probe: every
/// named binary plus a versioned libkrunfw.
fn has_required_files(dir: &Path) -> bool { has_named_files(dir) && find_libkrunfw(dir).is_ok() }

// ---------------------------------------------------------------------------
// Stage to destination
// ---------------------------------------------------------------------------

/// Copy required files from `source` to `dest`, set unix permissions to
/// match boxlite's expectations, then write a `.complete` stamp.
///
/// Writing the stamp last preserves the "atomic enough" guarantee boxlite
/// itself relies on — partial copies leave no stamp, so a re-run will
/// retry instead of silently using a half-staged dir.
fn stage_runtime(source: &Path, dest: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dest)?;

    let libkrunfw_name =
        find_libkrunfw(source).map_err(|e| std::io::Error::other(e.to_string()))?;

    // Build the file list: 4 named binaries + the versioned libkrunfw.
    let mut to_copy: Vec<String> = REQUIRED_NAMED_FILES
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    to_copy.push(libkrunfw_name);

    for name in &to_copy {
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
            let mode = if EXECUTABLE_FILES.contains(&name.as_str()) {
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::{
        net::SocketAddr,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use axum::{Router, body::Body, http::StatusCode, response::Response, routing::get};
    use tempfile::tempdir;
    use tokio::net::TcpListener;

    use super::*;

    // -----------------------------------------------------------------
    // Test fixture: build a tarball matching the upstream layout.
    // -----------------------------------------------------------------

    /// Files to put inside the synthesised tarball.
    struct FixtureContents {
        /// Set to `false` to omit `boxlite-guest` from the archive (used
        /// by the "missing required file" scenario).
        include_boxlite_guest: bool,
    }

    impl Default for FixtureContents {
        fn default() -> Self {
            Self {
                include_boxlite_guest: true,
            }
        }
    }

    /// Build a `.tar.gz` mirroring the upstream
    /// `boxlite-runtime-vX.Y.Z-<target>.tar.gz` layout: a top-level
    /// `boxlite-runtime/` directory holding the runtime files.
    fn build_fixture_tarball(contents: &FixtureContents) -> Vec<u8> {
        let mut buf: Vec<u8> = Vec::new();
        {
            let gz = flate2::write::GzEncoder::new(&mut buf, flate2::Compression::fast());
            let mut tar = tar::Builder::new(gz);

            let mut add = |name: &str, body: &[u8], mode: u32| {
                let mut header = tar::Header::new_gnu();
                header.set_size(body.len() as u64);
                header.set_mode(mode);
                header.set_cksum();
                tar.append_data(&mut header, format!("boxlite-runtime/{name}"), body)
                    .expect("append fixture entry");
            };

            add("boxlite-shim", b"#!fake shim\n", 0o755);
            if contents.include_boxlite_guest {
                add("boxlite-guest", b"#!fake guest\n", 0o755);
            }
            add("mke2fs", b"#!fake mke2fs\n", 0o755);
            add("debugfs", b"#!fake debugfs\n", 0o755);
            // Versioned libkrunfw — match the platform suffix.
            #[cfg(target_os = "macos")]
            add("libkrunfw.5.dylib", b"fake libkrunfw\n", 0o644);
            #[cfg(target_os = "linux")]
            add("libkrunfw.so.5", b"fake libkrunfw\n", 0o644);

            tar.into_inner()
                .expect("close tar builder")
                .finish()
                .expect("close gz encoder");
        }
        buf
    }

    /// Hermetic in-process HTTP server that serves a single fixed payload
    /// and counts hits. Bound to `127.0.0.1:0`.
    struct FixtureServer {
        url:       String,
        hits:      Arc<AtomicUsize>,
        _shutdown: tokio::sync::oneshot::Sender<()>,
    }

    impl FixtureServer {
        async fn start(payload: Vec<u8>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind ephemeral port");
            let addr: SocketAddr = listener.local_addr().expect("local addr");
            let hits = Arc::new(AtomicUsize::new(0));

            let payload = Arc::new(payload);
            let hits_for_handler = hits.clone();
            let app = Router::new().route(
                "/runtime.tar.gz",
                get(move || {
                    let payload = payload.clone();
                    let hits = hits_for_handler.clone();
                    async move {
                        hits.fetch_add(1, Ordering::SeqCst);
                        Response::builder()
                            .status(StatusCode::OK)
                            .header("content-type", "application/gzip")
                            .header("content-length", payload.len().to_string())
                            .body(Body::from((*payload).clone()))
                            .unwrap()
                    }
                }),
            );

            let (tx, rx) = tokio::sync::oneshot::channel::<()>();
            tokio::spawn(async move {
                let server = axum::serve(listener, app).with_graceful_shutdown(async move {
                    let _ = rx.await;
                });
                let _ = server.await;
            });

            Self {
                url: format!("http://{addr}/runtime.tar.gz"),
                hits,
                _shutdown: tx,
            }
        }
    }

    // -----------------------------------------------------------------
    // Scenario tests (BDD selectors in spec map to these names).
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn fresh_setup_downloads_and_stages_all_files() {
        let server = FixtureServer::start(build_fixture_tarball(&FixtureContents::default())).await;
        let dest_dir = tempdir().unwrap();
        let opts = SetupOptions {
            url:  server.url.clone(),
            dest: dest_dir.path().to_path_buf(),
        };

        let outcome = run_boxlite_setup_with(false, &opts)
            .await
            .expect("staging should succeed against the fixture server");

        match outcome {
            StageOutcome::Staged { dest } => assert_eq!(dest, opts.dest),
            StageOutcome::CheckOnly { .. } => panic!("expected Staged, got CheckOnly"),
        }

        for name in REQUIRED_NAMED_FILES {
            assert!(
                dest_dir.path().join(name).is_file(),
                "missing staged file {name}"
            );
        }
        let libkrunfw = find_libkrunfw(dest_dir.path()).expect("staged libkrunfw present");
        #[cfg(target_os = "macos")]
        assert_eq!(libkrunfw, "libkrunfw.5.dylib");
        #[cfg(target_os = "linux")]
        assert_eq!(libkrunfw, "libkrunfw.so.5");

        assert!(
            dest_dir.path().join(COMPLETE_STAMP).is_file(),
            ".complete stamp must be written"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for name in EXECUTABLE_FILES {
                let mode = fs::metadata(dest_dir.path().join(name))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777;
                assert_eq!(mode, 0o755, "{name} should be 0o755");
            }
            let lib_mode = fs::metadata(dest_dir.path().join(&libkrunfw))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(lib_mode, 0o644, "libkrunfw should be 0o644");
        }
    }

    #[tokio::test]
    async fn idempotent_skip_when_already_complete() {
        let server = FixtureServer::start(build_fixture_tarball(&FixtureContents::default())).await;
        let dest_dir = tempdir().unwrap();

        // Pre-populate destination with a valid staging layout.
        for name in REQUIRED_NAMED_FILES {
            fs::write(dest_dir.path().join(name), b"existing").unwrap();
        }
        #[cfg(target_os = "macos")]
        fs::write(dest_dir.path().join("libkrunfw.5.dylib"), b"existing").unwrap();
        #[cfg(target_os = "linux")]
        fs::write(dest_dir.path().join("libkrunfw.so.5"), b"existing").unwrap();
        fs::write(dest_dir.path().join(COMPLETE_STAMP), BOXLITE_VERSION).unwrap();

        let original_bytes = fs::read(dest_dir.path().join("boxlite-shim")).unwrap();
        let opts = SetupOptions {
            url:  server.url.clone(),
            dest: dest_dir.path().to_path_buf(),
        };

        let outcome = run_boxlite_setup_with(false, &opts)
            .await
            .expect("idempotent re-run should succeed");
        assert!(matches!(outcome, StageOutcome::Staged { .. }));

        // No HTTP request was made.
        assert_eq!(
            server.hits.load(Ordering::SeqCst),
            0,
            "server must not be hit on idempotent run"
        );
        // Existing files are unchanged byte-for-byte.
        let after_bytes = fs::read(dest_dir.path().join("boxlite-shim")).unwrap();
        assert_eq!(original_bytes, after_bytes);
    }

    #[tokio::test]
    async fn check_only_is_pure_dry_run() {
        let server = FixtureServer::start(build_fixture_tarball(&FixtureContents::default())).await;
        let dest_dir = tempdir().unwrap();
        let opts = SetupOptions {
            url:  server.url.clone(),
            dest: dest_dir.path().to_path_buf(),
        };

        let outcome = run_boxlite_setup_with(true, &opts)
            .await
            .expect("--check should succeed");

        match outcome {
            StageOutcome::CheckOnly { url, dest } => {
                assert_eq!(url, opts.url);
                assert_eq!(dest, opts.dest);
            }
            StageOutcome::Staged { .. } => panic!("expected CheckOnly, got Staged"),
        }
        assert_eq!(server.hits.load(Ordering::SeqCst), 0);
        assert!(!dest_dir.path().join(COMPLETE_STAMP).exists());
        assert!(fs::read_dir(dest_dir.path()).unwrap().next().is_none());
    }

    #[tokio::test]
    async fn missing_required_file_in_tarball_errors_cleanly() {
        let server = FixtureServer::start(build_fixture_tarball(&FixtureContents {
            include_boxlite_guest: false,
        }))
        .await;
        let dest_dir = tempdir().unwrap();
        let opts = SetupOptions {
            url:  server.url.clone(),
            dest: dest_dir.path().to_path_buf(),
        };

        let err = run_boxlite_setup_with(false, &opts)
            .await
            .expect_err("staging must fail when a required file is absent");
        let msg = format!("{err}");
        assert!(
            msg.contains("boxlite-guest"),
            "error message must name the missing file, got: {msg}"
        );
        assert!(
            !dest_dir.path().join(COMPLETE_STAMP).exists(),
            "no stamp must be written on failure"
        );
    }

    #[test]
    fn unsupported_platform_errors_cleanly() {
        let err = resolve_target("freebsd", "x86_64").expect_err("freebsd is unsupported");
        assert!(err.contains("freebsd"));
        assert!(err.contains("x86_64"));

        // Sanity: every supported pair resolves.
        assert!(resolve_target("macos", "aarch64").is_ok());
        assert!(resolve_target("linux", "x86_64").is_ok());
        assert!(resolve_target("linux", "aarch64").is_ok());
    }

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

    #[tokio::test]
    async fn target_dir_is_never_consulted() {
        // Lay down a `target/release/build/boxlite-deadbeef/out/runtime/`
        // populated with garbage. The new pipeline must ignore it
        // entirely — staging proceeds purely from the downloaded tarball.
        let scratch = tempdir().unwrap();
        let bogus_runtime = scratch
            .path()
            .join("target")
            .join("release")
            .join("build")
            .join("boxlite-deadbeef")
            .join("out")
            .join("runtime");
        fs::create_dir_all(&bogus_runtime).unwrap();
        for name in REQUIRED_NAMED_FILES {
            fs::write(bogus_runtime.join(name), b"GARBAGE").unwrap();
        }

        let server = FixtureServer::start(build_fixture_tarball(&FixtureContents::default())).await;
        let dest_dir = tempdir().unwrap();
        let opts = SetupOptions {
            url:  server.url.clone(),
            // Destination intentionally lives inside the scratch dir so
            // any "consulted target/" bug would be visible.
            dest: dest_dir.path().to_path_buf(),
        };

        run_boxlite_setup_with(false, &opts)
            .await
            .expect("staging should succeed independent of target/");

        // Staged content must come from the fixture (real signature),
        // not from `target/` (would be the literal "GARBAGE" payload).
        let shim = fs::read(dest_dir.path().join("boxlite-shim")).unwrap();
        assert_ne!(shim, b"GARBAGE", "staged file must not come from target/");
        assert_eq!(server.hits.load(Ordering::SeqCst), 1);
    }

    // -----------------------------------------------------------------
    // Supplementary unit tests for helpers.
    // -----------------------------------------------------------------

    #[test]
    fn libkrunfw_pattern_recognises_versioned_soname() {
        #[cfg(target_os = "macos")]
        {
            assert!(is_libkrunfw_soname("libkrunfw.5.dylib"));
            assert!(is_libkrunfw_soname("libkrunfw.42.dylib"));
            assert!(!is_libkrunfw_soname("libkrunfw.dylib"));
            assert!(!is_libkrunfw_soname("libkrunfw.5.so"));
            assert!(!is_libkrunfw_soname("libkrunfw.x.dylib"));
        }
        #[cfg(target_os = "linux")]
        {
            assert!(is_libkrunfw_soname("libkrunfw.so.5"));
            assert!(is_libkrunfw_soname("libkrunfw.so.5.0"));
            assert!(!is_libkrunfw_soname("libkrunfw.so"));
            assert!(!is_libkrunfw_soname("libkrunfw.5.dylib"));
        }
    }
}
