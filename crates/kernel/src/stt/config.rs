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
}

fn default_model() -> String { "whisper-1".to_owned() }

fn default_timeout_secs() -> u64 { 60 }
