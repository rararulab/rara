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

//! Template rendering — combines frontmatter, state, and body into the final
//! soul prompt string.

use crate::{file::SoulFile, state::SoulState};

/// Minimum trait confidence to include in the rendered prompt.
const TRAIT_CONFIDENCE_THRESHOLD: f32 = 0.6;

/// Render a soul file + optional state into the final prompt string.
///
/// The output format:
/// ```text
/// # Identity: {name}
///
/// Personality: {traits joined}
/// Current mood: {mood} (confidence: {confidence})
/// Relationship stage: {stage}
/// Emerged traits: {high-confidence traits}
/// Interests: {interests}
///
/// {markdown body}
///
/// ## Runtime Style Parameters
/// Formality: {n}/10
/// Verbosity: {n}/10
/// Humor: {n}/10
/// ```
pub fn render(soul: &SoulFile, state: Option<&SoulState>) -> String {
    let fm = &soul.frontmatter;
    let mut out = String::with_capacity(2048);

    // Identity header
    out.push_str(&format!("# Identity: {}\n\n", fm.name));

    // Personality traits
    if !fm.personality.is_empty() {
        out.push_str(&format!("Personality: {}\n", fm.personality.join(", ")));
    }

    // State-dependent sections
    if let Some(st) = state {
        out.push_str(&format!(
            "Current mood: {:?} (confidence: {:.1})\n",
            st.mood.current, st.mood.confidence
        ));
        out.push_str(&format!(
            "Relationship stage: {:?}\n",
            st.relationship_stage
        ));

        // High-confidence emerged traits
        let strong_traits: Vec<&str> = st
            .emerged_traits
            .iter()
            .filter(|t| t.confidence >= TRAIT_CONFIDENCE_THRESHOLD)
            .map(|t| t.r#trait.as_str())
            .collect();
        if !strong_traits.is_empty() {
            out.push_str(&format!("Emerged traits: {}\n", strong_traits.join(", ")));
        }

        // Discovered interests
        if !st.discovered_interests.is_empty() {
            out.push_str(&format!(
                "Interests: {}\n",
                st.discovered_interests.join(", ")
            ));
        }
    }

    out.push('\n');

    // Markdown body
    let body = soul.body.trim();
    if !body.is_empty() {
        out.push_str(body);
        out.push('\n');
    }

    // Runtime style parameters (only if state is present)
    if let Some(st) = state {
        out.push_str(&format!(
            "\n## Runtime Style Parameters\nFormality: {}/10\nVerbosity: {}/10\nHumor: {}/10\n",
            st.style_drift.formality, st.style_drift.verbosity, st.style_drift.humor_frequency,
        ));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        file::SoulFile,
        state::{EmergedTrait, MoodLabel, SoulState},
    };

    #[test]
    fn render_without_state() {
        let content = "---\nname: rara\npersonality:\n- warm\n- curious\n---\n## \
                       Background\n\nSome background.\n";
        let soul = SoulFile::parse(content).unwrap();
        let rendered = render(&soul, None);

        assert!(rendered.contains("# Identity: rara"));
        assert!(rendered.contains("Personality: warm, curious"));
        assert!(rendered.contains("## Background"));
        assert!(!rendered.contains("Runtime Style Parameters"));
    }

    #[test]
    fn render_with_state() {
        let content = "---\nname: rara\npersonality:\n- warm\n---\n## Body\n\nHello.\n";
        let soul = SoulFile::parse(content).unwrap();

        let mut state = SoulState::default();
        state.update_mood(MoodLabel::Cheerful, 0.8);
        state.discovered_interests.push("Rust".to_string());
        state.emerged_traits.push(EmergedTrait {
            r#trait:    "喜欢深夜聊技术".to_string(),
            confidence: 0.9,
            first_seen: None,
        });
        state.emerged_traits.push(EmergedTrait {
            r#trait:    "low confidence trait".to_string(),
            confidence: 0.3,
            first_seen: None,
        });

        let rendered = render(&soul, Some(&state));

        assert!(rendered.contains("Current mood: Cheerful"));
        assert!(rendered.contains("Interests: Rust"));
        assert!(rendered.contains("喜欢深夜聊技术"));
        assert!(!rendered.contains("low confidence trait"));
        assert!(rendered.contains("Formality: 5/10"));
    }
}
