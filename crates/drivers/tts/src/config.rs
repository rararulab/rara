use bon::Builder;
use serde::{Deserialize, Serialize};

/// Configuration for the Text-to-Speech service.
///
/// All fields are loaded from `config.yaml` — no hardcoded defaults in Rust.
///
/// ```yaml
/// tts:
///   base_url: "https://api.openai.com/v1"
///   api_key: "sk-..."
///   model: "tts-1"
///   voice: "alloy"
///   format: "opus"
///   speed: 1.0              # optional, 0.25-4.0
///   timeout_secs: 30        # optional
///   max_text_length: 4096   # optional
/// ```
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct TtsConfig {
    /// Base URL of the TTS API (e.g. `https://api.openai.com/v1`).
    pub base_url: String,

    /// Bearer token for authentication. Optional for self-hosted servers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// Model identifier (e.g. `"tts-1"`, `"tts-1-hd"`).
    pub model: String,

    /// Voice name (e.g. `"alloy"`, `"echo"`, `"fable"`, `"onyx"`, `"nova"`,
    /// `"shimmer"`).
    pub voice: String,

    /// Output audio format (e.g. `"opus"`, `"mp3"`, `"aac"`, `"flac"`).
    pub format: String,

    /// Speech speed multiplier (0.25-4.0). When `None`, server default is used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<f32>,

    /// Timeout per synthesis request in seconds. When `None`, reqwest default
    /// is used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,

    /// Maximum allowed input text length. When `None`, no client-side limit is
    /// enforced.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_text_length: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_deserializes_from_yaml() {
        let yaml = r#"
base_url: "https://api.openai.com/v1"
api_key: "sk-test-key"
model: "tts-1"
voice: "alloy"
format: "opus"
speed: 1.5
timeout_secs: 30
max_text_length: 4096
"#;
        let config: TtsConfig = serde_yaml::from_str(yaml).expect("failed to parse YAML");
        assert_eq!(config.base_url, "https://api.openai.com/v1");
        assert_eq!(config.api_key.as_deref(), Some("sk-test-key"));
        assert_eq!(config.model, "tts-1");
        assert_eq!(config.voice, "alloy");
        assert_eq!(config.format, "opus");
        assert_eq!(config.speed, Some(1.5));
        assert_eq!(config.timeout_secs, Some(30));
        assert_eq!(config.max_text_length, Some(4096));
    }

    #[test]
    fn config_deserializes_minimal_yaml() {
        let yaml = r#"
base_url: "http://localhost:8080/v1"
model: "tts-1"
voice: "nova"
format: "mp3"
"#;
        let config: TtsConfig = serde_yaml::from_str(yaml).expect("failed to parse YAML");
        assert_eq!(config.base_url, "http://localhost:8080/v1");
        assert!(config.api_key.is_none());
        assert_eq!(config.voice, "nova");
        assert_eq!(config.format, "mp3");
        assert!(config.speed.is_none());
        assert!(config.timeout_secs.is_none());
        assert!(config.max_text_length.is_none());
    }

    #[test]
    fn config_round_trips_through_serde() {
        let config = TtsConfig::builder()
            .base_url("https://api.openai.com/v1".to_owned())
            .model("tts-1-hd".to_owned())
            .voice("shimmer".to_owned())
            .format("flac".to_owned())
            .speed(2.0)
            .build();

        let yaml = serde_yaml::to_string(&config).expect("failed to serialize");
        let parsed: TtsConfig = serde_yaml::from_str(&yaml).expect("failed to deserialize");

        assert_eq!(parsed.base_url, config.base_url);
        assert_eq!(parsed.model, config.model);
        assert_eq!(parsed.voice, config.voice);
        assert_eq!(parsed.format, config.format);
        assert_eq!(parsed.speed, config.speed);
    }
}
