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

use snafu::{ResultExt, Whatever};

use super::{
    prompt::{self, SetupMode},
    whisper_install, writer,
};

/// STT configuration result collected during the setup wizard.
pub struct SttResult {
    /// whisper-server base URL.
    pub base_url:   String,
    /// Model identifier.
    pub model:      String,
    /// Optional language hint (e.g. "zh", "en").
    pub language:   Option<String>,
    /// Path to the whisper-server binary (managed mode only).
    pub server_bin: Option<String>,
    /// Path to the GGML model file (managed mode only).
    pub model_path: Option<String>,
    /// Whether rara should start/stop the whisper-server process.
    pub managed:    bool,
}

/// Guide the user through optional STT (speech-to-text) configuration.
///
/// STT is entirely optional — the wizard only enters this section if the user
/// explicitly opts in.  A best-effort connectivity check is performed against
/// the whisper-server URL.
pub async fn setup_stt(_mode: SetupMode) -> Result<Option<SttResult>, Whatever> {
    prompt::print_step("STT (optional)");

    if !prompt::confirm("Configure speech-to-text?", false) {
        return Ok(None);
    }

    let base_url = prompt::ask("whisper-server URL", Some("http://localhost:8080"));
    let model = prompt::ask("Model", Some("whisper-1"));

    let lang = prompt::ask("Language hint (e.g. zh, en; empty for auto-detect)", None);
    let language = if lang.is_empty() { None } else { Some(lang) };

    // Best-effort connectivity check
    match validate_stt(&base_url).await {
        Ok(()) => prompt::print_ok("whisper-server connected"),
        Err(e) => prompt::print_err(&format!(
            "cannot reach server: {e} (you can start it later)"
        )),
    }

    Ok(Some(SttResult {
        base_url,
        model,
        language,
        server_bin: None,
        model_path: None,
        managed: false,
    }))
}

/// Standalone `setup whisper` — detects, installs, configures, and verifies
/// whisper.cpp, then writes STT settings into the config file.
pub async fn run_whisper_setup() -> Result<(), Whatever> {
    println!("rara setup whisper\n");

    // Run the full detection / install / verification pipeline.
    let install = whisper_install::ensure_whisper().await?;

    // Build the STT config result.
    let base_url = format!("http://127.0.0.1:{}", install.port);

    let lang = prompt::ask("Language hint (e.g. zh, en; empty for auto-detect)", None);
    let language = if lang.is_empty() { None } else { Some(lang) };

    let stt_result = SttResult {
        base_url,
        model: "whisper-1".to_owned(),
        language,
        server_bin: Some(install.server_bin.to_string_lossy().into_owned()),
        model_path: Some(install.model_path.to_string_lossy().into_owned()),
        managed: true,
    };

    // Merge into existing config file.
    let config_path = rara_paths::config_file();
    let base_yaml: Option<serde_yaml::Value> = std::fs::read_to_string(&config_path)
        .ok()
        .and_then(|s| serde_yaml::from_str(&s).ok());

    let config_value =
        writer::assemble_config(base_yaml, None, None, None, None, Some(&stt_result))?;

    let yaml = writer::to_yaml(&config_value)?;

    println!("\n═══ STT Config Preview ═══");
    println!("{}", writer::mask_secrets(&yaml));

    if prompt::confirm("Write config?", true) {
        writer::write_config(config_path, &yaml)?;
    } else {
        println!("Aborted. Config not written.");
    }

    println!("\n═══ Whisper setup complete ═══");
    println!("\nWhisper-server will start automatically with rara (managed: true).");
    println!("To manage the server yourself, set `stt.managed: false` in config.yaml.");

    Ok(())
}

/// Best-effort connectivity check to whisper-server.
async fn validate_stt(base_url: &str) -> Result<(), Whatever> {
    let client = reqwest::Client::new();
    client
        .get(base_url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .whatever_context("connection failed")?;
    Ok(())
}
