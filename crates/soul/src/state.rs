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

//! Soul runtime state — mood, relationship, emerged traits, style drift.

use std::path::Path;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use snafu::ResultExt;

use crate::error::{IoSnafu, ParseStateSnafu, Result, SerializeStateSnafu};

/// Maximum history entries kept in state file.
const MAX_HISTORY_ENTRIES: usize = 10;

/// Runtime soul state, persisted to `soul-state.yaml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SoulState {
    #[serde(default)]
    pub mood:                 Mood,
    #[serde(default = "default_relationship")]
    pub relationship_stage:   RelationshipStage,
    #[serde(default)]
    pub emerged_traits:       Vec<EmergedTrait>,
    #[serde(default)]
    pub style_drift:          StyleDrift,
    #[serde(default)]
    pub discovered_interests: Vec<String>,
    #[serde(default)]
    pub history:              Vec<HistoryEntry>,
}

fn default_relationship() -> RelationshipStage { RelationshipStage::Stranger }

/// Current mood state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mood {
    #[serde(default = "default_mood_label")]
    pub current:    MoodLabel,
    #[serde(default = "default_confidence")]
    pub confidence: f32,
    #[serde(default)]
    pub updated_at: Option<Timestamp>,
}

impl Default for Mood {
    fn default() -> Self {
        Self {
            current:    MoodLabel::Calm,
            confidence: 0.5,
            updated_at: None,
        }
    }
}

fn default_mood_label() -> MoodLabel { MoodLabel::Calm }
fn default_confidence() -> f32 { 0.5 }

/// Recognized mood labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MoodLabel {
    Calm,
    Cheerful,
    Focused,
    Playful,
    Tired,
}

/// Relationship progression stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationshipStage {
    Stranger,
    Acquaintance,
    Friend,
    CloseFriend,
}

/// A personality trait that emerged through interaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmergedTrait {
    pub r#trait:    String,
    #[serde(default = "default_confidence")]
    pub confidence: f32,
    pub first_seen: Option<String>,
}

/// Style drift parameters (1-10 scale).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StyleDrift {
    #[serde(default = "default_style_5")]
    pub formality:       u8,
    #[serde(default = "default_style_5")]
    pub verbosity:       u8,
    #[serde(default = "default_style_5")]
    pub humor_frequency: u8,
}

impl Default for StyleDrift {
    fn default() -> Self {
        Self {
            formality:       5,
            verbosity:       5,
            humor_frequency: 5,
        }
    }
}

fn default_style_5() -> u8 { 5 }

/// A history entry recording a soul state change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub timestamp:   Timestamp,
    pub r#type:      String,
    pub description: String,
}

impl Default for SoulState {
    fn default() -> Self {
        Self {
            mood:                 Mood::default(),
            relationship_stage:   RelationshipStage::Stranger,
            emerged_traits:       vec![],
            style_drift:          StyleDrift::default(),
            discovered_interests: vec![],
            history:              vec![],
        }
    }
}

impl SoulState {
    /// Load state from a YAML file. Returns `Ok(None)` if the file does not
    /// exist.
    pub fn load(path: &Path) -> Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(path).context(IoSnafu {
            path: path.to_path_buf(),
        })?;
        let state: Self = serde_yaml::from_str(&content).context(ParseStateSnafu)?;
        Ok(Some(state))
    }

    /// Save state to a YAML file, creating parent directories as needed.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context(IoSnafu {
                path: parent.to_path_buf(),
            })?;
        }
        let yaml = serde_yaml::to_string(self).context(SerializeStateSnafu)?;
        let content = format!("# Auto-generated soul state — do not edit manually\n\n{yaml}");
        std::fs::write(path, content).context(IoSnafu {
            path: path.to_path_buf(),
        })?;
        Ok(())
    }

    /// Append a history entry, keeping at most [`MAX_HISTORY_ENTRIES`].
    pub fn append_history(&mut self, entry: HistoryEntry) {
        self.history.push(entry);
        if self.history.len() > MAX_HISTORY_ENTRIES {
            let excess = self.history.len() - MAX_HISTORY_ENTRIES;
            self.history.drain(..excess);
        }
    }

    /// Update mood, recording the change timestamp.
    pub fn update_mood(&mut self, label: MoodLabel, confidence: f32) {
        self.mood.current = label;
        self.mood.confidence = confidence.clamp(0.0, 1.0);
        self.mood.updated_at = Some(Timestamp::now());
    }

    /// Clamp style drift formality within the given boundaries.
    pub fn clamp_formality(&mut self, min: Option<u8>, max: Option<u8>) {
        if let Some(lo) = min {
            if self.style_drift.formality < lo {
                self.style_drift.formality = lo;
            }
        }
        if let Some(hi) = max {
            if self.style_drift.formality > hi {
                self.style_drift.formality = hi;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_is_neutral() {
        let state = SoulState::default();
        assert_eq!(state.mood.current, MoodLabel::Calm);
        assert_eq!(state.relationship_stage, RelationshipStage::Stranger);
        assert_eq!(state.style_drift.formality, 5);
        assert!(state.emerged_traits.is_empty());
    }

    #[test]
    fn history_cap() {
        let mut state = SoulState::default();
        for i in 0..15 {
            state.append_history(HistoryEntry {
                timestamp:   Timestamp::now(),
                r#type:      "test".to_string(),
                description: format!("entry {i}"),
            });
        }
        assert_eq!(state.history.len(), MAX_HISTORY_ENTRIES);
        assert!(state.history[0].description.contains("entry 5"));
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("soul-state.yaml");

        let mut state = SoulState::default();
        state.update_mood(MoodLabel::Cheerful, 0.9);
        state.discovered_interests.push("Rust".to_string());
        state.save(&path).unwrap();

        let loaded = SoulState::load(&path).unwrap().unwrap();
        assert_eq!(loaded.mood.current, MoodLabel::Cheerful);
        assert_eq!(loaded.discovered_interests, vec!["Rust"]);
    }

    #[test]
    fn load_nonexistent_returns_none() {
        let result = SoulState::load(Path::new("/nonexistent/soul-state.yaml")).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn clamp_formality() {
        let mut state = SoulState::default();
        state.style_drift.formality = 1;
        state.clamp_formality(Some(3), Some(8));
        assert_eq!(state.style_drift.formality, 3);

        state.style_drift.formality = 10;
        state.clamp_formality(Some(3), Some(8));
        assert_eq!(state.style_drift.formality, 8);
    }
}
