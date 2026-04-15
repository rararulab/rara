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
        vec![KeyboardButton::new("\u{1f195} New Session")],
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
            let pct = if limit > 0 {
                (f64::from(input_tokens) / f64::from(limit) * 100.0) as u32
            } else {
                0
            };
            format!("\u{1f4ca} {used}/{limit_str} ({pct}%)")
        }
        None => format!("\u{1f4ca} {used}"),
    }
}

fn format_model_button(model: &str) -> String {
    let display: String = model.chars().take(25).collect();
    if display.is_empty() {
        "\u{1f916} (no model)".to_string()
    } else {
        format!("\u{1f916} {display}")
    }
}

/// Format a token count as a compact human-readable string.
///
/// Duplicated from `adapter::format_token_count` because that function is
/// `pub(super)` and cannot be called from a sibling module.
fn format_token_count(tokens: u32) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", f64::from(tokens) / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}k", f64::from(tokens) / 1_000.0)
    } else {
        format!("{tokens}")
    }
}

// ── Button press detection ──────────────────────────────────────────────

/// Returns `true` if the message text matches a context-usage button press.
pub fn is_context_button(text: &str) -> bool { text.starts_with("\u{1f4ca} ") }

/// Returns `true` if the message text matches a model-name button press.
pub fn is_model_button(text: &str) -> bool { text.starts_with("\u{1f916} ") }

/// Returns `true` if the message text matches the "New Session" button press.
pub fn is_new_session_button(text: &str) -> bool { text.contains("New Session") }

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
    fn model_button_formatting() {
        let kb = build_main_keyboard(0, None, "claude-sonnet-4");
        assert_eq!(kb.keyboard[0][1].text, "\u{1f916} claude-sonnet-4");
    }

    #[test]
    fn model_button_truncation() {
        let long_model = "a]".repeat(20);
        let kb = build_main_keyboard(0, None, &long_model);
        // 25 chars max + emoji prefix
        assert!(kb.keyboard[0][1].text.chars().count() <= 25 + 3);
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
        assert!(is_new_session_button("\u{1f195} New Session"));
        // Negative cases
        assert!(!is_context_button("hello"));
        assert!(!is_model_button("hello"));
        assert!(!is_new_session_button("hello"));
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
