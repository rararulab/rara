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

//! Anchor summary data type.
//!
//! [`AnchorSummary`] is a lightweight view of a persisted
//! [`Anchor`](super::TapEntryKind::Anchor) entry, pairing the anchor's name
//! (e.g. `session/start`) with the arbitrary structured state captured at that
//! checkpoint.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Rendered anchor summary, matching Bub's public anchor model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnchorSummary {
    /// Anchor label such as `session/start` or a handoff name.
    pub name:  String,
    /// Arbitrary structured state captured at the anchor point.
    pub state: Value,
}

/// Strongly-typed handoff state contract per tape.systems spec.
///
/// All fields are optional for backward compatibility — anchors written before
/// this type was introduced may lack some fields.  When creating *new* anchors,
/// callers should populate at least `summary` and `owner`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffState {
    /// Current phase label (e.g. "discovery", "implement", "verify").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// Human-readable summary of what happened before this anchor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Actionable items for the next phase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_steps: Option<String>,
    /// IDs of key source entries that this anchor summarizes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_ids: Vec<u64>,
    /// Who created this anchor ("human", "agent", "system").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Free-form extension data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<Value>,
}
