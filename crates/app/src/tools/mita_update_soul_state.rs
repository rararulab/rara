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

//! Mita-exclusive tool for updating an agent's soul state fields.
//!
//! Allows Mita to update macro-level soul state fields:
//! - `relationship_stage`
//! - `emerged_traits`
//! - `style_drift`
//! - `discovered_interests`

use async_trait::async_trait;
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_soul::state::{EmergedTrait, HistoryEntry, RelationshipStage, StyleDrift};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::info;

use super::notify::push_notification;

/// Valid field names that can be updated.
const UPDATABLE_FIELDS: &[&str] = &[
    "relationship_stage",
    "emerged_traits",
    "style_drift",
    "discovered_interests",
];

/// Input parameters for the update-soul-state tool.
///
/// The `value` field is untyped (JSON value) because its schema depends on
/// which `field` is being updated.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateSoulStateParams {
    /// Target agent name (e.g. "rara").
    agent: String,
    /// The soul state field to update.
    field: String,
    /// The new value for the field (schema depends on field).
    value: Value,
}

/// Mita-exclusive tool: update a specific field in an agent's soul state.
#[derive(ToolDef)]
#[tool(
    name = "update-soul-state",
    description = "Update a specific field in an agent's soul state. Fields:\n- \
                   relationship_stage: one of \"stranger\", \"acquaintance\", \"friend\", \
                   \"close_friend\"\n- emerged_traits: array of {\"trait\": \"...\", \
                   \"confidence\": 0.0-1.0, \"first_seen\": \"...\"}\n- style_drift: \
                   {\"formality\": 1-10, \"verbosity\": 1-10, \"humor_frequency\": 1-10}\n- \
                   discovered_interests: array of strings",
    bypass_interceptor
)]
pub struct UpdateSoulStateTool;

impl UpdateSoulStateTool {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl ToolExecute for UpdateSoulStateTool {
    type Output = Value;
    type Params = UpdateSoulStateParams;

    async fn run(
        &self,
        params: UpdateSoulStateParams,
        context: &ToolContext,
    ) -> anyhow::Result<Value> {
        if !UPDATABLE_FIELDS.contains(&params.field.as_str()) {
            anyhow::bail!(
                "invalid field '{}': must be one of {}",
                params.field,
                UPDATABLE_FIELDS.join(", ")
            );
        }

        // Load existing state or create default.
        let mut state = rara_soul::loader::load_state(&params.agent)
            .map_err(|e| anyhow::anyhow!("failed to load soul state: {e}"))?
            .unwrap_or_default();

        match params.field.as_str() {
            "relationship_stage" => {
                let stage: RelationshipStage = serde_json::from_value(params.value.clone())
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "invalid relationship_stage value: {e}. Expected one of: stranger, \
                             acquaintance, friend, close_friend"
                        )
                    })?;
                let old = state.relationship_stage;
                state.relationship_stage = stage;
                state.append_history(HistoryEntry {
                    timestamp:   jiff::Timestamp::now(),
                    r#type:      "relationship_stage_change".to_string(),
                    description: format!("{old:?} -> {stage:?}"),
                });
                info!(
                    agent = %params.agent,
                    ?old,
                    ?stage,
                    "soul state: relationship stage updated"
                );
            }
            "emerged_traits" => {
                let traits: Vec<EmergedTrait> = serde_json::from_value(params.value.clone())
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "invalid emerged_traits value: {e}. Expected array of {{\"trait\": \
                             \"...\", \"confidence\": 0.0-1.0, \"first_seen\": \"...\"}}"
                        )
                    })?;
                let count = traits.len();
                // Merge: add new traits, update confidence for existing ones.
                for new_trait in traits {
                    if let Some(existing) = state
                        .emerged_traits
                        .iter_mut()
                        .find(|t| t.r#trait == new_trait.r#trait)
                    {
                        existing.confidence = new_trait.confidence;
                    } else {
                        state.emerged_traits.push(new_trait);
                    }
                }
                state.append_history(HistoryEntry {
                    timestamp:   jiff::Timestamp::now(),
                    r#type:      "emerged_traits_update".to_string(),
                    description: format!(
                        "merged {count} trait(s), total: {}",
                        state.emerged_traits.len()
                    ),
                });
                info!(
                    agent = %params.agent,
                    count,
                    total = state.emerged_traits.len(),
                    "soul state: emerged traits updated"
                );
            }
            "style_drift" => {
                let drift: StyleDrift =
                    serde_json::from_value(params.value.clone()).map_err(|e| {
                        anyhow::anyhow!(
                            "invalid style_drift value: {e}. Expected {{\"formality\": 1-10, \
                             \"verbosity\": 1-10, \"humor_frequency\": 1-10}}"
                        )
                    })?;
                state.style_drift = drift;
                state.append_history(HistoryEntry {
                    timestamp:   jiff::Timestamp::now(),
                    r#type:      "style_drift_update".to_string(),
                    description: format!(
                        "formality={}, verbosity={}, humor={}",
                        state.style_drift.formality,
                        state.style_drift.verbosity,
                        state.style_drift.humor_frequency
                    ),
                });
                info!(agent = %params.agent, ?state.style_drift, "soul state: style drift updated");
            }
            "discovered_interests" => {
                let interests: Vec<String> =
                    serde_json::from_value(params.value.clone()).map_err(|e| {
                        anyhow::anyhow!(
                            "invalid discovered_interests value: {e}. Expected array of strings"
                        )
                    })?;
                let count = interests.len();
                // Merge: add new interests, skip duplicates.
                for interest in interests {
                    if !state.discovered_interests.contains(&interest) {
                        state.discovered_interests.push(interest);
                    }
                }
                state.append_history(HistoryEntry {
                    timestamp:   jiff::Timestamp::now(),
                    r#type:      "discovered_interests_update".to_string(),
                    description: format!(
                        "merged {count} interest(s), total: {}",
                        state.discovered_interests.len()
                    ),
                });
                info!(
                    agent = %params.agent,
                    count,
                    total = state.discovered_interests.len(),
                    "soul state: discovered interests updated"
                );
            }
            _ => unreachable!(),
        }

        // Persist the updated state.
        rara_soul::loader::save_state(&params.agent, &state)
            .map_err(|e| anyhow::anyhow!("failed to save soul state: {e}"))?;

        push_notification(
            context,
            format!(
                "\u{2699}\u{fe0f} Soul state updated: {}.{}",
                params.agent, params.field
            ),
        );

        Ok(json!({
            "status": "ok",
            "agent": params.agent,
            "field": params.field,
            "message": format!("Soul state field '{}' updated for agent '{}'.", params.field, params.agent)
        }))
    }
}
