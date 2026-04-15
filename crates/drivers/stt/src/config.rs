use std::path::PathBuf;

use bon::Builder;
use serde::{Deserialize, Serialize};

/// Configuration for the Speech-to-Text service.
///
/// When present in `config.yaml`, `base_url` is **required** — the
/// application will refuse to start if it is empty or missing.
///
/// ```yaml
/// stt:
///   base_url: "http://localhost:8080"
///   model: "whisper-1"       # optional
///   language: "zh"           # optional, auto-detect if omitted
///   timeout_secs: 60         # optional, default 60
///   correction:              # optional LLM post-correction
///     enabled: true
///     model: "glm-4-flash"
///     provider: "glm"
/// ```
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct SttConfig {
    /// Base URL of the whisper-server (e.g. `http://localhost:8080`).
    pub base_url:     String,
    /// Model identifier sent to the server (default: `whisper-1`).
    #[serde(default = "default_model")]
    #[builder(default = default_model())]
    pub model:        String,
    /// Optional BCP-47 language hint (e.g. `"zh"`, `"en"`).
    /// When omitted the server auto-detects the language.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language:     Option<String>,
    /// Timeout per transcription request in seconds (default: 60).
    #[serde(default = "default_timeout_secs")]
    #[builder(default = default_timeout_secs())]
    pub timeout_secs: u64,

    /// Whether rara should spawn and supervise the whisper-server process.
    /// When `false` (default), the user manages the server externally.
    #[serde(default)]
    #[builder(default)]
    pub managed: bool,

    /// Path to the whisper-server binary (required when `managed: true`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_bin: Option<PathBuf>,

    /// Path to the whisper model file (required when `managed: true`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_path: Option<PathBuf>,

    /// Optional LLM correction pass after transcription.
    ///
    /// When enabled, the raw transcription is sent through a fast LLM to
    /// fix obvious speech-recognition errors before delivery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correction: Option<SttCorrectionConfig>,
}

/// Configuration for the optional LLM post-correction pass.
///
/// When `enabled` is `true`, the raw STT output is sent through a fast LLM
/// that fixes obvious transcription errors while preserving the original
/// meaning. Correction failure never blocks the message — the adapter falls
/// back to the raw transcription.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttCorrectionConfig {
    /// Whether to run an LLM correction pass after transcription.
    pub enabled:  bool,
    /// The LLM model to use for correction (e.g. `"glm-4-flash"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model:    Option<String>,
    /// The LLM provider to use (e.g. `"glm"`). Falls back to the default
    /// driver when omitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

fn default_model() -> String { "whisper-1".to_owned() }

fn default_timeout_secs() -> u64 { 60 }
