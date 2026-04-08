//! STT service — HTTP client for OpenAI-compatible transcription endpoints.

use reqwest::multipart;
use snafu::ResultExt;
use tracing::instrument;

use crate::{
    SttConfig,
    error::{self, HttpSnafu, ParseSnafu, Result},
};

/// Maximum number of retries for transient failures.
const MAX_RETRIES: u32 = 1;

/// Delay between retries — matches the whisper supervisor's restart delay.
const RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(2);

/// HTTP client for an OpenAI-compatible `/v1/audio/transcriptions` endpoint
/// (e.g. whisper.cpp server).
#[derive(Debug, Clone, bon::Builder)]
pub struct SttService {
    client:   reqwest::Client,
    base_url: String,
    model:    String,
    language: Option<String>,
}

/// Result of a transcription request.
#[derive(Debug, serde::Deserialize)]
struct TranscriptionResponse {
    text: String,
}

impl SttService {
    /// Build a new service from config.
    pub fn from_config(config: &SttConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .expect("failed to build reqwest client for STT");

        Self {
            client,
            base_url: config.base_url.trim_end_matches('/').to_owned(),
            model: config.model.clone(),
            language: config.language.clone(),
        }
    }

    /// Transcribe raw audio bytes into text.
    ///
    /// `mime_type` should be the audio MIME type (e.g. `"audio/ogg"`,
    /// `"audio/mpeg"`). The file extension is inferred from the MIME type
    /// for the multipart form field.
    ///
    /// Transient failures (timeout, 429, 5xx) are retried once after a 2 s
    /// delay before returning an error.
    #[instrument(skip_all, fields(audio_len = audio_data.len(), mime_type))]
    pub async fn transcribe(&self, audio_data: Vec<u8>, mime_type: &str) -> Result<String> {
        let mut last_err = None;

        for attempt in 0..=MAX_RETRIES {
            if attempt > 0 {
                tracing::info!(
                    attempt,
                    "retrying STT transcription after transient failure"
                );
                tokio::time::sleep(RETRY_DELAY).await;
            }

            match self.transcribe_once(&audio_data, mime_type).await {
                Ok(text) => return Ok(text),
                Err(e) if e.is_transient() && attempt < MAX_RETRIES => {
                    tracing::warn!(error = %e, attempt, "transient STT error, will retry");
                    last_err = Some(e);
                }
                Err(e) => return Err(e),
            }
        }

        // Unreachable in practice — the loop always returns — but the
        // compiler cannot prove it.
        Err(last_err.expect("retry loop must have recorded an error"))
    }

    /// Single-shot transcription attempt (no retry logic).
    async fn transcribe_once(&self, audio_data: &[u8], mime_type: &str) -> Result<String> {
        let ext = extension_from_mime(mime_type);
        let filename = format!("voice.{ext}");

        let file_part = multipart::Part::bytes(audio_data.to_vec())
            .file_name(filename)
            .mime_str(mime_type)
            // mime_str only fails on invalid MIME — treat as HTTP-level error.
            .map_err(|e| error::SttError::Http { source: e })?;

        let mut form = multipart::Form::new()
            .part("file", file_part)
            .text("model", self.model.clone());

        if let Some(ref lang) = self.language {
            form = form.text("language", lang.clone());
        }

        let url = format!("{}/v1/audio/transcriptions", self.base_url);

        let resp = self
            .client
            .post(&url)
            .multipart(form)
            .send()
            .await
            .context(HttpSnafu)?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(error::SttError::ServerError { status, body });
        }

        let result: TranscriptionResponse = resp.json().await.context(ParseSnafu)?;

        if result.text.trim().is_empty() {
            return Err(error::SttError::EmptyResponse);
        }

        Ok(result.text)
    }
}

/// Map audio MIME type to file extension for the multipart filename.
fn extension_from_mime(mime: &str) -> &'static str {
    match mime {
        "audio/ogg" | "audio/opus" => "ogg",
        "audio/mpeg" => "mp3",
        "audio/wav" | "audio/x-wav" => "wav",
        "audio/mp4" | "audio/m4a" => "m4a",
        "audio/flac" => "flac",
        _ => "ogg",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_to_extension_mapping() {
        assert_eq!(extension_from_mime("audio/ogg"), "ogg");
        assert_eq!(extension_from_mime("audio/opus"), "ogg");
        assert_eq!(extension_from_mime("audio/mpeg"), "mp3");
        assert_eq!(extension_from_mime("audio/wav"), "wav");
        assert_eq!(extension_from_mime("audio/mp4"), "m4a");
        assert_eq!(extension_from_mime("audio/flac"), "flac");
        assert_eq!(extension_from_mime("audio/unknown"), "ogg");
    }
}
