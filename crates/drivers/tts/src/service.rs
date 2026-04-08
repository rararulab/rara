//! TTS service — HTTP client for OpenAI-compatible speech synthesis endpoints.

use serde_json::json;
use snafu::ResultExt;
use tracing::instrument;

use crate::{
    config::TtsConfig,
    error::{self, Result},
};

/// HTTP client for an OpenAI-compatible `/v1/audio/speech` endpoint.
pub struct TtsService {
    client: reqwest::Client,
    config: TtsConfig,
}

/// Raw audio output returned by [`TtsService::synthesize`].
#[derive(Debug)]
pub struct AudioOutput {
    /// Audio bytes in the requested format.
    pub data:      Vec<u8>,
    /// MIME type corresponding to the output format (e.g. `"audio/mpeg"`).
    pub mime_type: String,
}

impl TtsService {
    /// Build a new service from config.
    pub fn from_config(config: &TtsConfig) -> Self {
        let mut builder = reqwest::Client::builder();

        if let Some(secs) = config.timeout_secs {
            builder = builder.timeout(std::time::Duration::from_secs(secs));
        }

        let client = builder
            .build()
            .expect("failed to build reqwest client for TTS");

        Self {
            client,
            config: config.clone(),
        }
    }

    /// Synthesize speech from text using the default voice.
    #[instrument(skip_all, fields(text_len = text.len()))]
    pub async fn synthesize(&self, text: &str) -> Result<AudioOutput> {
        self.synthesize_with_voice(text, &self.config.voice).await
    }

    /// Synthesize speech from text using a specific voice override.
    #[instrument(skip_all, fields(text_len = text.len(), voice))]
    pub async fn synthesize_with_voice(&self, text: &str, voice: &str) -> Result<AudioOutput> {
        // 1. Check max_text_length
        if let Some(max) = self.config.max_text_length {
            snafu::ensure!(
                text.len() <= max,
                error::TextTooLongSnafu {
                    max,
                    actual: text.len(),
                }
            );
        }

        // 2. Build JSON body
        let mut body = json!({
            "model": self.config.model,
            "input": text,
            "voice": voice,
            "response_format": self.config.format,
        });

        if let Some(speed) = self.config.speed {
            body["speed"] = json!(speed);
        }

        // 3. Build request
        let url = format!(
            "{}/audio/speech",
            self.config.base_url.trim_end_matches('/')
        );

        let mut req = self.client.post(&url).json(&body);

        if let Some(ref api_key) = self.config.api_key {
            req = req.bearer_auth(api_key);
        }

        // 4. Send and read response
        let resp = req.send().await.context(error::HttpSnafu)?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(error::ServerSnafu { status, body }.build());
        }

        let data = resp.bytes().await.context(error::HttpSnafu)?.to_vec();

        Ok(AudioOutput {
            data,
            mime_type: format_to_mime(&self.config.format),
        })
    }
}

/// Map the OpenAI response_format name to a MIME type.
fn format_to_mime(format: &str) -> String {
    match format {
        "opus" => "audio/ogg;codecs=opus".to_owned(),
        "mp3" => "audio/mpeg".to_owned(),
        "aac" => "audio/aac".to_owned(),
        "flac" => "audio/flac".to_owned(),
        _ => format!("audio/{format}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_to_mime_mapping() {
        assert_eq!(format_to_mime("opus"), "audio/ogg;codecs=opus");
        assert_eq!(format_to_mime("mp3"), "audio/mpeg");
        assert_eq!(format_to_mime("aac"), "audio/aac");
        assert_eq!(format_to_mime("flac"), "audio/flac");
        assert_eq!(format_to_mime("wav"), "audio/wav");
    }

    #[test]
    fn text_too_long_is_rejected() {
        let config = TtsConfig::builder()
            .base_url("http://localhost".to_owned())
            .model("tts-1".to_owned())
            .voice("alloy".to_owned())
            .format("opus".to_owned())
            .max_text_length(10_usize)
            .build();

        let service = TtsService::from_config(&config);

        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();

        let err = rt
            .block_on(service.synthesize("this text is definitely longer than ten characters"))
            .unwrap_err();

        assert!(
            err.to_string().contains("text exceeds max length"),
            "unexpected error: {err}"
        );
    }
}
