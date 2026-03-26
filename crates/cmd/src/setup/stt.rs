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

use super::prompt::{self, SetupMode};

/// STT configuration result.
pub struct SttResult {
    /// whisper-server base URL.
    pub base_url: String,
    /// Model identifier.
    pub model:    String,
    /// Optional language hint (e.g. "zh", "en").
    pub language: Option<String>,
}

/// Guide the user through optional STT (speech-to-text) configuration.
///
/// STT is entirely optional — the wizard only enters this section if the user
/// explicitly opts in.  A best-effort connectivity check is performed against
/// the whisper-server URL.
pub async fn setup_stt(mode: SetupMode) -> Result<Option<SttResult>, Whatever> {
    prompt::print_step("STT (optional)");

    // STT config type lives on the STT feature branch and may not be available
    // on main yet, so we cannot detect existing config.  In FillMissing mode we
    // still offer the step since we have no way to know if it was configured.
    let _ = mode;

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
    }))
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
