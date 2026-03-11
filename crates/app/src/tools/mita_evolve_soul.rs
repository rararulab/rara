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

//! Mita-exclusive tool for triggering soul evolution.
//!
//! Orchestrates the full evolution pipeline:
//! 1. Load current soul file + state
//! 2. Validate sufficient signal for evolution
//! 3. Snapshot current soul
//! 4. (Placeholder) Generate evolved soul via LLM
//! 5. Validate boundaries
//! 6. Write new soul file
//! 7. Return evolution summary

use async_trait::async_trait;
use rara_kernel::tool::{AgentTool, ToolContext, ToolOutput};
use serde_json::json;
use tracing::info;

use rara_soul::state::StyleDrift;

/// Minimum number of emerged traits required to trigger evolution.
const MIN_EMERGED_TRAITS: usize = 3;

/// Mita-exclusive tool: trigger soul.md evolution for an agent.
pub struct EvolveSoulTool;

impl EvolveSoulTool {
    pub fn new() -> Self { Self }
}

/// Check whether the soul state has accumulated enough signal to warrant
/// evolution. Returns `None` if ready, or `Some(reason)` if not.
fn check_evolution_readiness(
    state: &rara_soul::state::SoulState,
) -> Option<String> {
    let default_drift = StyleDrift::default();
    let has_drift = state.style_drift.formality != default_drift.formality
        || state.style_drift.verbosity != default_drift.verbosity
        || state.style_drift.humor_frequency != default_drift.humor_frequency;

    let trait_count = state.emerged_traits.len();

    if trait_count < MIN_EMERGED_TRAITS && !has_drift {
        Some(format!(
            "Not enough signal to evolve: {} emerged trait(s) (need {}+) and no style drift",
            trait_count, MIN_EMERGED_TRAITS
        ))
    } else {
        None
    }
}

#[async_trait]
impl AgentTool for EvolveSoulTool {
    fn name(&self) -> &str { "evolve-soul" }

    fn description(&self) -> &str {
        "Trigger soul.md evolution for an agent. Checks whether enough signal has \
         accumulated (emerged traits, style drift), snapshots the current soul, and \
         initiates evolution. Currently uses a placeholder for the LLM rewrite step."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "agent": {
                    "type": "string",
                    "description": "Target agent name (e.g. \"rara\")"
                }
            },
            "required": ["agent"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let agent = params
            .get("agent")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: agent"))?;

        // 1. Load current soul file.
        let loaded = rara_soul::load_soul(agent, None)
            .map_err(|e| anyhow::anyhow!("failed to load soul file: {e}"))?
            .ok_or_else(|| anyhow::anyhow!(
                "no soul file found for agent '{agent}'"
            ))?;

        let soul = loaded.soul;
        let current_version = soul.frontmatter.version;

        // 2. Load current state.
        let state = rara_soul::loader::load_state(agent)
            .map_err(|e| anyhow::anyhow!("failed to load soul state: {e}"))?
            .unwrap_or_default();

        // 3. Check evolution readiness.
        if let Some(reason) = check_evolution_readiness(&state) {
            info!(agent, %reason, "soul evolution skipped: insufficient signal");
            return Ok(json!({
                "status": "skipped",
                "agent": agent,
                "reason": reason,
                "emerged_traits_count": state.emerged_traits.len(),
                "style_drift": {
                    "formality": state.style_drift.formality,
                    "verbosity": state.style_drift.verbosity,
                    "humor_frequency": state.style_drift.humor_frequency
                }
            })
            .into());
        }

        // 4. Create snapshot of current soul.
        let snapshots_dir = rara_soul::loader::snapshots_dir(agent);
        let snapshot_path = rara_soul::create_snapshot(&soul, &snapshots_dir)
            .map_err(|e| anyhow::anyhow!("failed to create soul snapshot: {e}"))?;

        info!(
            agent,
            version = current_version,
            snapshot = %snapshot_path.display(),
            "soul snapshot created before evolution"
        );

        // 5. Placeholder: LLM-driven evolution not yet implemented.
        //
        // When implemented, this will call `SoulEvolver::propose_evolution()`
        // to generate a new soul.md based on the accumulated state changes.
        // For now, we record that evolution was requested and return a pending
        // status so Mita knows to retry later.
        let summary = format!(
            "Evolution pending for agent '{agent}' (v{current_version}). \
             Snapshot saved at {}. \
             Signal: {} emerged trait(s), style_drift=({},{},{}), \
             {} discovered interest(s), relationship={:?}. \
             LLM-driven soul rewrite not yet implemented.",
            snapshot_path.display(),
            state.emerged_traits.len(),
            state.style_drift.formality,
            state.style_drift.verbosity,
            state.style_drift.humor_frequency,
            state.discovered_interests.len(),
            state.relationship_stage,
        );

        info!(agent, %summary, "soul evolution: pending LLM implementation");

        Ok(json!({
            "status": "pending",
            "agent": agent,
            "current_version": current_version,
            "snapshot_path": snapshot_path.display().to_string(),
            "signal": {
                "emerged_traits_count": state.emerged_traits.len(),
                "emerged_traits": state.emerged_traits.iter()
                    .map(|t| t.r#trait.clone())
                    .collect::<Vec<_>>(),
                "style_drift": {
                    "formality": state.style_drift.formality,
                    "verbosity": state.style_drift.verbosity,
                    "humor_frequency": state.style_drift.humor_frequency
                },
                "discovered_interests_count": state.discovered_interests.len(),
                "relationship_stage": format!("{:?}", state.relationship_stage)
            },
            "message": summary
        })
        .into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rara_soul::state::{EmergedTrait, SoulState};

    #[test]
    fn readiness_default_state_not_ready() {
        let state = SoulState::default();
        let result = check_evolution_readiness(&state);
        assert!(result.is_some());
        assert!(result.unwrap().contains("Not enough signal"));
    }

    #[test]
    fn readiness_enough_traits() {
        let mut state = SoulState::default();
        for i in 0..3 {
            state.emerged_traits.push(EmergedTrait {
                r#trait:    format!("trait_{i}"),
                confidence: 0.8,
                first_seen: None,
            });
        }
        assert!(check_evolution_readiness(&state).is_none());
    }

    #[test]
    fn readiness_style_drift_only() {
        let mut state = SoulState::default();
        state.style_drift.formality = 8; // deviated from default 5
        assert!(check_evolution_readiness(&state).is_none());
    }

    #[test]
    fn readiness_few_traits_no_drift_not_ready() {
        let mut state = SoulState::default();
        state.emerged_traits.push(EmergedTrait {
            r#trait:    "curious".to_string(),
            confidence: 0.7,
            first_seen: None,
        });
        assert!(check_evolution_readiness(&state).is_some());
    }
}
