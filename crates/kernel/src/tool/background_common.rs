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

//! Shared logic for spawning and registering background agents.
//!
//! Used by both `TaskTool` and `SpawnBackgroundTool` to avoid duplicating
//! the post-manifest spawn sequence.

use std::time::Duration;

use tracing::{info, warn};

use crate::{
    agent::AgentManifest,
    handle::KernelHandle,
    io::{AgentEvent, StreamEvent},
    session::{BackgroundTaskEntry, SessionKey},
    tool::ToolContext,
};

/// Spawn a child agent from a manifest, register it as a background task,
/// and return a JSON status payload.
///
/// Both `TaskTool` and `SpawnBackgroundTool` delegate here after building
/// their respective `AgentManifest`.
pub(crate) async fn spawn_and_register_background(
    handle: &KernelHandle,
    session_key: &SessionKey,
    mut manifest: AgentManifest,
    input: String,
    context: &ToolContext,
) -> anyhow::Result<serde_json::Value> {
    // Append structured-output instructions so the background agent
    // self-summarizes before returning results to the parent.
    manifest
        .system_prompt
        .push_str(crate::agent::STRUCTURED_OUTPUT_SUFFIX);

    // Resolve principal from parent session.
    let principal = handle
        .process_table()
        .with(session_key, |proc| proc.principal.clone())
        .ok_or_else(|| anyhow::anyhow!("parent session not found: {}", session_key))?;

    info!(
        parent = %session_key,
        agent = %manifest.name,
        description = %manifest.description,
        "spawning background agent"
    );

    let agent_handle = handle
        .spawn_child(session_key, &principal, manifest.clone(), input)
        .await
        .map_err(|e| anyhow::anyhow!("spawn failed: {e}"))?;

    let child_key = agent_handle.session_key;

    // Register as background task on parent session.
    handle.register_background_task(
        session_key,
        BackgroundTaskEntry {
            child_key,
            agent_name: manifest.name.clone(),
            description: manifest.description.clone(),
            created_at: jiff::Timestamp::now(),
            trigger_message_id: context.rara_message_id.clone(),
        },
    );

    // Emit BackgroundTaskStarted to parent's active streams so clients
    // can display an ongoing status indicator with elapsed timer.
    handle.stream_hub().emit_to_session(
        session_key,
        StreamEvent::BackgroundTaskStarted {
            task_id:     child_key.to_string(),
            agent_name:  manifest.name.clone(),
            description: manifest.description.clone(),
        },
    );

    // Spawn watcher to drain result_rx with a timeout so it cannot hang forever.
    let watcher_timeout = Duration::from_secs(manifest.worker_timeout_secs.unwrap_or(600) + 60);
    let child_key_for_log = child_key;
    tokio::spawn(async move {
        let mut rx = agent_handle.result_rx;
        let result = tokio::time::timeout(watcher_timeout, async {
            while let Some(event) = rx.recv().await {
                if matches!(event, AgentEvent::Done(_)) {
                    break;
                }
            }
        })
        .await;
        if result.is_err() {
            warn!(
                child = %child_key_for_log,
                timeout_secs = watcher_timeout.as_secs(),
                "background agent watcher timed out — child may still be running"
            );
        }
    });

    Ok(serde_json::json!({
        "task_id": child_key.to_string(),
        "agent_name": manifest.name,
        "status": "spawned",
        "message": "Background agent is now running. Results will be delivered when complete."
    }))
}
