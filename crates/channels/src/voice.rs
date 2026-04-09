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

//! Voice transcription post-processing utilities.
//!
//! Two layers of quality improvement for STT output:
//!
//! - **Layer A (annotation)**: Always prepends a hint so the downstream LLM
//!   knows the text came from speech recognition and may contain errors.
//! - **Layer B (correction)**: Optionally runs a fast LLM pass to fix obvious
//!   transcription mistakes before delivery. Controlled by
//!   [`SttCorrectionConfig`].

use rara_kernel::llm::{
    DriverRegistryRef,
    types::{CompletionRequest, Message, ToolChoice},
};
use rara_stt::SttCorrectionConfig;

/// Prefix prepended to every voice transcription so the downstream LLM
/// interprets the text with appropriate error tolerance.
pub const VOICE_ANNOTATION_PREFIX: &str =
    "[Voice transcription \u{2014} may contain errors, interpret by context]";

/// Annotate a transcribed text with the voice-transcription hint.
pub fn annotate_voice(text: &str) -> String { format!("{VOICE_ANNOTATION_PREFIX}\n{text}") }

/// Run an optional LLM correction pass on the raw STT output.
///
/// Returns the corrected text when correction is enabled and succeeds.
/// Falls back to the original `text` on any error — correction failure
/// must never block the message.
pub async fn maybe_correct(
    text: &str,
    correction: Option<&SttCorrectionConfig>,
    driver_registry: Option<&DriverRegistryRef>,
) -> String {
    let Some(cfg) = correction.filter(|c| c.enabled) else {
        return text.to_owned();
    };
    let Some(registry) = driver_registry else {
        tracing::debug!("STT correction enabled but no driver registry available, skipping");
        return text.to_owned();
    };

    let (driver, model) = match registry.resolve(
        "_stt_correction",
        cfg.provider.as_deref(),
        cfg.model.as_deref(),
    ) {
        Ok(pair) => pair,
        Err(e) => {
            tracing::warn!(error = %e, "STT correction: failed to resolve LLM driver, skipping");
            return text.to_owned();
        }
    };

    let request = CompletionRequest {
        model,
        messages: vec![
            Message::system(
                "You are a transcription error corrector. Fix obvious speech recognition \
                 mistakes. Output only the corrected text.",
            ),
            Message::user(format!(
                "Correct any transcription errors in the following voice message. Preserve the \
                 original meaning. Only fix obvious mistakes. Output the corrected text only, no \
                 explanation.\n\n{text}"
            )),
        ],
        tools: Vec::new(),
        temperature: Some(0.1),
        max_tokens: Some(4096),
        thinking: None,
        tool_choice: ToolChoice::None,
        parallel_tool_calls: false,
        frequency_penalty: None,
        top_p: None,
    };

    match driver.complete(request).await {
        Ok(resp) => resp.content.unwrap_or_else(|| text.to_owned()),
        Err(e) => {
            tracing::warn!(error = %e, "STT correction LLM call failed, using raw transcription");
            text.to_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annotation_format() {
        let result = annotate_voice("hello world");
        assert!(result.starts_with(VOICE_ANNOTATION_PREFIX));
        assert!(result.ends_with("hello world"));
        // Exactly one newline between prefix and text.
        let expected = format!("{VOICE_ANNOTATION_PREFIX}\nhello world");
        assert_eq!(result, expected);
    }

    #[test]
    fn annotation_preserves_multiline() {
        let input = "line one\nline two";
        let result = annotate_voice(input);
        assert_eq!(
            result,
            format!("{VOICE_ANNOTATION_PREFIX}\nline one\nline two")
        );
    }

    #[tokio::test]
    async fn correction_disabled_returns_original() {
        let text = "some text";
        // No correction config.
        assert_eq!(maybe_correct(text, None, None).await, text);

        // Config present but disabled.
        let cfg = SttCorrectionConfig {
            enabled:  false,
            model:    None,
            provider: None,
        };
        assert_eq!(maybe_correct(text, Some(&cfg), None).await, text);
    }

    #[tokio::test]
    async fn correction_enabled_no_registry_returns_original() {
        let cfg = SttCorrectionConfig {
            enabled:  true,
            model:    Some("test-model".to_owned()),
            provider: Some("test".to_owned()),
        };
        assert_eq!(
            maybe_correct("raw text", Some(&cfg), None).await,
            "raw text"
        );
    }
}
