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

//! Reply Keyboard for Telegram — persistent buttons below the input field.
//!
//! Provides a two-row keyboard layout:
//! - Row 1: context usage gauge + active model name
//! - Row 2: "New Session" quick action
//!
//! The keyboard is attached via [`build_main_keyboard`] and remains visible
//! (sticky) until replaced by another `ReplyMarkup`.  Button presses arrive
//! as regular text messages; the `is_*_button` helpers detect them so the
//! adapter can intercept before ingesting.

use teloxide::types::{KeyboardButton, KeyboardMarkup};

use super::adapter::format_token_count;

/// Label emitted by the "New Session" Reply Keyboard button.
///
/// Exposed so the adapter can send it as-is and so detection logic can use
/// exact-match comparison instead of substring search.
pub const NEW_SESSION_BUTTON: &str = "\u{1f195} New Session";

const CONTEXT_BUTTON_PREFIX: &str = "\u{1f4ca} ";
const MODEL_BUTTON_PREFIX: &str = "\u{1f916} ";
const MODEL_NAME_MAX_CHARS: usize = 25;

/// Build the main Reply Keyboard with session status buttons.
///
/// Layout:
/// ```text
/// [ 📊 context_usage ]  [ 🤖 model_name ]
/// [ 🆕 New Session ]
/// ```
pub fn build_main_keyboard(
    input_tokens: u32,
    context_limit: Option<u32>,
    model: &str,
) -> KeyboardMarkup {
    let context_text = format_context_button(input_tokens, context_limit);
    let model_text = format_model_button(model);

    KeyboardMarkup::new(vec![
        vec![
            KeyboardButton::new(context_text),
            KeyboardButton::new(model_text),
        ],
        vec![KeyboardButton::new(NEW_SESSION_BUTTON)],
    ])
    .resize_keyboard()
    .persistent()
}

// ── Formatting helpers ──────────────────────────────────────────────────

fn format_context_button(input_tokens: u32, context_limit: Option<u32>) -> String {
    let used = format_token_count(input_tokens);
    match context_limit {
        Some(limit) => {
            let limit_str = format_token_count(limit);
            // Clamp to 0..=100 so over-quota turns do not surface odd values
            // like "(142%)" in the button label.
            let pct = if limit > 0 {
                ((f64::from(input_tokens) / f64::from(limit) * 100.0).round() as u32).min(100)
            } else {
                0
            };
            format!("{CONTEXT_BUTTON_PREFIX}{used}/{limit_str} ({pct}%)")
        }
        None => format!("{CONTEXT_BUTTON_PREFIX}{used}"),
    }
}

/// Truncate to [`MODEL_NAME_MAX_CHARS`] code points. Model identifiers in
/// practice are ASCII (e.g. `claude-sonnet-4`, `openai/gpt-5-codex`), so
/// `chars().take(...)` is safe — no grapheme-cluster concerns.
fn format_model_button(model: &str) -> String {
    let display: String = model.chars().take(MODEL_NAME_MAX_CHARS).collect();
    if display.is_empty() {
        format!("{MODEL_BUTTON_PREFIX}(no model)")
    } else {
        format!("{MODEL_BUTTON_PREFIX}{display}")
    }
}

// ── Button press detection ──────────────────────────────────────────────

/// Returns `true` if the message text matches a context-usage button press.
pub fn is_context_button(text: &str) -> bool { text.starts_with(CONTEXT_BUTTON_PREFIX) }

/// Returns `true` if the message text matches a model-name button press.
pub fn is_model_button(text: &str) -> bool { text.starts_with(MODEL_BUTTON_PREFIX) }

/// Returns `true` if the message text is exactly the "New Session" button.
///
/// Uses exact match (not `contains`) so free-form user messages that happen
/// to contain the phrase "New Session" do not get intercepted as button
/// presses and silently swallowed.
pub fn is_new_session_button(text: &str) -> bool { text == NEW_SESSION_BUTTON }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_layout_has_two_rows() {
        let kb = build_main_keyboard(1500, Some(200_000), "claude-sonnet-4");
        assert_eq!(kb.keyboard.len(), 2);
        assert_eq!(kb.keyboard[0].len(), 2);
        assert_eq!(kb.keyboard[1].len(), 1);
    }

    #[test]
    fn context_button_formatting() {
        let kb = build_main_keyboard(0, Some(200_000), "claude-sonnet-4");
        assert_eq!(kb.keyboard[0][0].text, "\u{1f4ca} 0/200.0k (0%)");

        let kb = build_main_keyboard(150_000, Some(200_000), "claude-sonnet-4");
        assert_eq!(kb.keyboard[0][0].text, "\u{1f4ca} 150.0k/200.0k (75%)");
    }

    #[test]
    fn context_percent_clamped_to_100() {
        // Over-quota should not produce (142%) or similar.
        let kb = build_main_keyboard(300_000, Some(200_000), "m");
        assert!(kb.keyboard[0][0].text.ends_with("(100%)"));
    }

    #[test]
    fn model_button_formatting() {
        let kb = build_main_keyboard(0, None, "claude-sonnet-4");
        assert_eq!(kb.keyboard[0][1].text, "\u{1f916} claude-sonnet-4");
    }

    #[test]
    fn model_button_truncation() {
        let long_model = "a]".repeat(20);
        let kb = build_main_keyboard(0, None, &long_model);
        // Content chars capped at MODEL_NAME_MAX_CHARS; the emoji + space
        // prefix is 2 chars.
        assert!(kb.keyboard[0][1].text.chars().count() <= MODEL_NAME_MAX_CHARS + 2);
    }

    #[test]
    fn empty_model_shows_placeholder() {
        let kb = build_main_keyboard(0, None, "");
        assert_eq!(kb.keyboard[0][1].text, "\u{1f916} (no model)");
    }

    #[test]
    fn button_detection() {
        assert!(is_context_button("\u{1f4ca} 0/200.0k (0%)"));
        assert!(is_model_button("\u{1f916} claude-sonnet-4"));
        assert!(is_new_session_button(NEW_SESSION_BUTTON));
        // Negative cases
        assert!(!is_context_button("hello"));
        assert!(!is_model_button("hello"));
        assert!(!is_new_session_button("hello"));
    }

    #[test]
    fn new_session_button_is_exact_match() {
        // Regression: free-form user text containing the phrase must not
        // be misclassified as a button press.
        assert!(!is_new_session_button(
            "Can we start a New Session about X?"
        ));
        assert!(!is_new_session_button("New Session"));
        assert!(!is_new_session_button("\u{1f195} New Session please"));
        assert!(is_new_session_button(NEW_SESSION_BUTTON));
    }

    #[test]
    fn keyboard_is_persistent_and_resized() {
        let kb = build_main_keyboard(0, None, "test");
        assert!(kb.is_persistent);
        assert!(kb.resize_keyboard);
    }

    #[test]
    fn no_context_limit() {
        let kb = build_main_keyboard(5000, None, "test");
        assert_eq!(kb.keyboard[0][0].text, "\u{1f4ca} 5.0k");
    }
}
