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

//! Automated whisper.cpp detection, installation, and verification.

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use snafu::{ResultExt, Whatever};

use super::prompt;

/// Default port for whisper-server during setup verification.
const DEFAULT_PORT: u16 = 8080;

/// Hugging Face base URL for whisper.cpp GGML models.
const MODEL_BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

/// Available whisper model sizes with approximate download sizes.
const MODEL_OPTIONS: &[(&str, &str)] = &[
    ("tiny", "~75 MB,  fastest, lowest accuracy"),
    ("base", "~142 MB, good balance for most use cases"),
    ("small", "~466 MB, better accuracy"),
    ("medium", "~1.5 GB, high accuracy"),
    ("large-v3-turbo", "~1.5 GB, best accuracy with turbo speed"),
];

/// Result of the whisper installation process.
pub struct WhisperInstallResult {
    /// Path to the whisper-server binary.
    pub server_bin: PathBuf,
    /// Path to the downloaded model file.
    pub model_path: PathBuf,
    /// Port the server should listen on.
    pub port:       u16,
}

/// Installation directory under rara's data dir.
fn whisper_dir() -> PathBuf { rara_paths::data_dir().join("whisper") }

/// Run the full whisper detection / installation / verification pipeline.
pub async fn ensure_whisper() -> Result<WhisperInstallResult, Whatever> {
    prompt::print_step("Whisper Installation");

    // 1. Check for existing whisper-server binary.
    let server_bin = match find_whisper_server() {
        Some(path) => {
            prompt::print_ok(&format!("found whisper-server at {}", path.display()));
            path
        }
        None => {
            println!("  whisper-server not found, installing...");
            install_whisper_server().await?
        }
    };

    // 2. Check for / download model.
    let model_path = ensure_model().await?;

    // 3. Choose port.
    let port_str = prompt::ask("whisper-server port", Some(&DEFAULT_PORT.to_string()));
    let port: u16 = port_str.parse().whatever_context("invalid port number")?;

    // 4. Test the server.
    test_server(&server_bin, &model_path, port).await?;

    Ok(WhisperInstallResult {
        server_bin,
        model_path,
        port,
    })
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// Search for an existing `whisper-server` binary in PATH and common locations.
fn find_whisper_server() -> Option<PathBuf> {
    // Check PATH first.
    if let Ok(output) = Command::new("which")
        .arg("whisper-server")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
    {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            if !path.is_empty() {
                return Some(PathBuf::from(path));
            }
        }
    }

    // Check our own install directory.
    let local_bin = whisper_dir().join("bin").join("whisper-server");
    if local_bin.is_file() {
        return Some(local_bin);
    }

    // Check Homebrew (macOS).
    let brew_path = PathBuf::from("/opt/homebrew/bin/whisper-server");
    if brew_path.is_file() {
        return Some(brew_path);
    }
    let brew_path_x86 = PathBuf::from("/usr/local/bin/whisper-server");
    if brew_path_x86.is_file() {
        return Some(brew_path_x86);
    }

    None
}

// ---------------------------------------------------------------------------
// Installation
// ---------------------------------------------------------------------------

/// Install whisper.cpp by building from source.
async fn install_whisper_server() -> Result<PathBuf, Whatever> {
    // Check build prerequisites.
    check_prerequisite("cmake")?;
    check_prerequisite("make")?;
    check_prerequisite("git")?;

    let base_dir = whisper_dir();
    let src_dir = base_dir.join("whisper.cpp");
    let build_dir = src_dir.join("build");
    let bin_dir = base_dir.join("bin");

    std::fs::create_dir_all(&base_dir).whatever_context("failed to create whisper directory")?;

    // Clone or update whisper.cpp.
    if src_dir.join("CMakeLists.txt").is_file() {
        println!("  updating whisper.cpp source...");
        run_cmd("git", &["pull", "--ff-only"], Some(&src_dir))?;
    } else {
        println!("  cloning whisper.cpp...");
        run_cmd(
            "git",
            &[
                "clone",
                "--depth=1",
                "https://github.com/ggml-org/whisper.cpp.git",
                &src_dir.to_string_lossy(),
            ],
            Some(&base_dir),
        )?;
    }

    // Build with cmake.
    println!("  building whisper-server (this may take a few minutes)...");
    std::fs::create_dir_all(&build_dir).whatever_context("failed to create build directory")?;

    run_cmd(
        "cmake",
        &["-B", "build", "-DCMAKE_BUILD_TYPE=Release"],
        Some(&src_dir),
    )?;
    run_cmd(
        "cmake",
        &["--build", "build", "--target", "whisper-server", "-j"],
        Some(&src_dir),
    )?;

    // Copy binary to our bin directory.
    std::fs::create_dir_all(&bin_dir).whatever_context("failed to create bin directory")?;

    let built_bin = build_dir.join("bin").join("whisper-server");
    if !built_bin.is_file() {
        snafu::whatever!(
            "build succeeded but whisper-server binary not found at {}",
            built_bin.display()
        );
    }

    let target_bin = bin_dir.join("whisper-server");
    std::fs::copy(&built_bin, &target_bin).whatever_context("failed to copy binary")?;

    // Make executable (unix).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&target_bin)
            .whatever_context("failed to read binary metadata")?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&target_bin, perms)
            .whatever_context("failed to set binary permissions")?;
    }

    prompt::print_ok(&format!(
        "installed whisper-server at {}",
        target_bin.display()
    ));
    Ok(target_bin)
}

/// Check that a command-line tool is available.
fn check_prerequisite(cmd: &str) -> Result<(), Whatever> {
    let status = Command::new("which")
        .arg(cmd)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .whatever_context(format!("failed to check for {cmd}"))?;

    if !status.success() {
        snafu::whatever!(
            "{cmd} is required but not found. Please install it first.\n  macOS: brew install \
             {cmd}\n  Ubuntu/Debian: sudo apt-get install {cmd}"
        );
    }
    Ok(())
}

/// Run a shell command, streaming output to stdout.
fn run_cmd(program: &str, args: &[&str], cwd: Option<&Path>) -> Result<(), Whatever> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }

    let status = cmd
        .status()
        .whatever_context(format!("failed to run {program} {}", args.join(" ")))?;

    if !status.success() {
        snafu::whatever!("{program} {} exited with {status}", args.join(" "));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Model download
// ---------------------------------------------------------------------------

/// Ensure a whisper model is available, downloading if needed.
async fn ensure_model() -> Result<PathBuf, Whatever> {
    prompt::print_step("Whisper Model");

    let models_dir = whisper_dir().join("models");
    std::fs::create_dir_all(&models_dir).whatever_context("failed to create models directory")?;

    // Show existing models.
    let existing: Vec<_> = std::fs::read_dir(&models_dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "bin"))
        .map(|e| e.path())
        .collect();

    if !existing.is_empty() {
        println!("  existing models:");
        for (i, p) in existing.iter().enumerate() {
            println!(
                "    {}. {}",
                i + 1,
                p.file_name().unwrap_or_default().to_string_lossy()
            );
        }
        if prompt::confirm("Use an existing model?", true) {
            let idx = if existing.len() == 1 {
                0
            } else {
                let labels: Vec<&str> = existing
                    .iter()
                    .map(|p| p.file_name().unwrap_or_default().to_str().unwrap_or("?"))
                    .collect();
                prompt::ask_choice("Select model:", &labels)
            };
            return Ok(existing[idx].clone());
        }
    }

    // Let user pick a model size to download.
    let labels: Vec<String> = MODEL_OPTIONS
        .iter()
        .map(|(name, desc)| format!("{name} — {desc}"))
        .collect();
    let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let choice = prompt::ask_choice("Select model to download:", &label_refs);
    let model_name = MODEL_OPTIONS[choice].0;

    let filename = format!("ggml-{model_name}.bin");
    let model_path = models_dir.join(&filename);

    if model_path.is_file() {
        prompt::print_ok(&format!("{filename} already exists"));
        return Ok(model_path);
    }

    let url = format!("{MODEL_BASE_URL}/{filename}");
    println!("  downloading {filename}...");

    download_file(&url, &model_path).await?;

    prompt::print_ok(&format!("downloaded {filename}"));
    Ok(model_path)
}

/// Download a file with progress indication.
async fn download_file(url: &str, dest: &Path) -> Result<(), Whatever> {
    // Prefer curl for better progress display.
    let status = Command::new("curl")
        .args(["-L", "--progress-bar", "-o"])
        .arg(dest.as_os_str())
        .arg(url)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status();

    if matches!(status, Ok(s) if s.success()) {
        return Ok(());
    }

    // Fallback to wget.
    let status = Command::new("wget")
        .args(["-q", "--show-progress", "-O"])
        .arg(dest.as_os_str())
        .arg(url)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status();

    if matches!(status, Ok(s) if s.success()) {
        return Ok(());
    }

    snafu::whatever!("failed to download {url} — install curl or wget")
}

// ---------------------------------------------------------------------------
// Server verification
// ---------------------------------------------------------------------------

/// Start whisper-server, verify it responds, then shut it down.
async fn test_server(server_bin: &Path, model_path: &Path, port: u16) -> Result<(), Whatever> {
    prompt::print_step("Verification");
    println!("  starting whisper-server on port {port} for testing...");

    // Check if something is already listening on this port.
    if port_in_use(port) {
        // Try to reach the existing server.
        let url = format!("http://127.0.0.1:{port}/health");
        let client = reqwest::Client::new();
        match client
            .get(&url)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                prompt::print_ok(&format!("whisper-server already running on port {port}"));
                return Ok(());
            }
            _ => {
                snafu::whatever!(
                    "port {port} is in use but does not appear to be a whisper-server"
                );
            }
        }
    }

    // Start the server process.
    // Use --inference-path to make it OpenAI-compatible with rara's SttService.
    let mut child = Command::new(server_bin)
        .args([
            "-m",
            &model_path.to_string_lossy(),
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--inference-path",
            "/v1/audio/transcriptions",
            "--convert",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .whatever_context("failed to start whisper-server")?;

    // Wait for the server to be ready (poll /health).
    let health_url = format!("http://127.0.0.1:{port}/health");
    let client = reqwest::Client::new();
    let mut ready = false;

    for i in 0..30 {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        // Check if the process exited early.
        if let Ok(Some(status)) = child.try_wait() {
            let stderr = child
                .stderr
                .take()
                .map(|mut s| {
                    let mut buf = String::new();
                    std::io::Read::read_to_string(&mut s, &mut buf).ok();
                    buf
                })
                .unwrap_or_default();
            snafu::whatever!("whisper-server exited prematurely with {status}\n{stderr}");
        }

        match client
            .get(&health_url)
            .timeout(std::time::Duration::from_secs(2))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                ready = true;
                break;
            }
            _ => {
                if i % 5 == 4 {
                    println!("  still waiting for server to load model...");
                }
            }
        }
    }

    if !ready {
        let _ = child.kill();
        snafu::whatever!("whisper-server did not become ready within 30 seconds");
    }

    prompt::print_ok("whisper-server is running and healthy");

    // Send a test transcription request with a minimal silent WAV.
    println!("  sending test transcription request...");
    match test_transcription(port).await {
        Ok(()) => prompt::print_ok("transcription test passed"),
        Err(e) => {
            prompt::print_err(&format!("transcription test failed: {e}"));
            prompt::print_err("voice messages may not work correctly");
        }
    }

    // Shut down the test server.
    println!("  stopping test server...");
    let _ = child.kill();
    let _ = child.wait();
    prompt::print_ok("test server stopped");

    Ok(())
}

/// Check if a TCP port is in use.
fn port_in_use(port: u16) -> bool { std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() }

/// Send a test transcription request using a minimal silent WAV file.
async fn test_transcription(port: u16) -> Result<(), Whatever> {
    let wav_data = generate_silent_wav();

    let file_part = reqwest::multipart::Part::bytes(wav_data)
        .file_name("test.wav")
        .mime_str("audio/wav")
        .whatever_context("invalid MIME type")?;

    let form = reqwest::multipart::Form::new()
        .part("file", file_part)
        .text("response_format", "json");

    let url = format!("http://127.0.0.1:{port}/v1/audio/transcriptions");
    let client = reqwest::Client::new();

    let resp = client
        .post(&url)
        .multipart(form)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .whatever_context("transcription request failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        snafu::whatever!("server returned {status}: {body}");
    }

    // Parse response — just verify it has a "text" field.
    let body: serde_json::Value = resp
        .json()
        .await
        .whatever_context("failed to parse response")?;

    if body.get("text").is_none() {
        snafu::whatever!("response missing 'text' field: {body}");
    }

    Ok(())
}

/// Generate a minimal 1-second silent 16-bit 16kHz mono WAV file in memory.
fn generate_silent_wav() -> Vec<u8> {
    let sample_rate: u32 = 16000;
    let bits_per_sample: u16 = 16;
    let num_channels: u16 = 1;
    let duration_secs: u32 = 1;
    let num_samples = sample_rate * duration_secs;
    let data_size = num_samples * (bits_per_sample as u32 / 8) * num_channels as u32;
    let file_size = 36 + data_size;

    let mut buf = Vec::with_capacity(file_size as usize + 8);

    // RIFF header
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&file_size.to_le_bytes());
    buf.extend_from_slice(b"WAVE");

    // fmt chunk
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes()); // chunk size
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM format
    buf.extend_from_slice(&num_channels.to_le_bytes());
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    let byte_rate = sample_rate * num_channels as u32 * bits_per_sample as u32 / 8;
    buf.extend_from_slice(&byte_rate.to_le_bytes());
    let block_align = num_channels * bits_per_sample / 8;
    buf.extend_from_slice(&block_align.to_le_bytes());
    buf.extend_from_slice(&bits_per_sample.to_le_bytes());

    // data chunk (all zeros = silence)
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_size.to_le_bytes());
    buf.resize(buf.len() + data_size as usize, 0);

    buf
}
