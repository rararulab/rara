//! STT service — HTTP client for OpenAI-compatible transcription endpoints.

use anyhow::Context;
use reqwest::multipart;
use tracing::instrument;

use super::SttConfig;

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
            .timeout(std::time::Duration::from_secs(60))
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
    #[instrument(skip_all, fields(audio_len = audio_data.len(), mime_type))]
    pub async fn transcribe(&self, audio_data: Vec<u8>, mime_type: &str) -> anyhow::Result<String> {
        let ext = extension_from_mime(mime_type);
        let filename = format!("voice.{ext}");

        let file_part = multipart::Part::bytes(audio_data)
            .file_name(filename)
            .mime_str(mime_type)
            .context("invalid MIME type")?;

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
            .context("STT request failed")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("STT server returned {status}: {body}");
        }

        let result: TranscriptionResponse =
            resp.json().await.context("failed to parse STT response")?;

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
