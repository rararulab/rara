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

//! Builder for the `scheduled_job` executor agent manifest.
//!
//! Unlike the static manifests in `rara-agents`, the scheduled-job executor
//! is parameterised per fired job — its system prompt embeds the job ID,
//! trigger schedule, task message, and routing tags so the LLM has the full
//! context of what to execute and how to report its result.
//!
//! Lives in `rara-kernel` rather than `rara-agents` because `rara-agents`
//! depends on `rara-kernel` (for [`AgentManifest`]); placing the builder
//! here avoids the circular dependency that would otherwise arise from
//! `kernel::handle_scheduled_task` needing to call into `rara-agents`.

use super::{AgentManifest, AgentRole, Priority};

/// Build the `scheduled_job` agent manifest for a specific scheduled task.
///
/// The returned manifest bakes the supplied `job_id`, `trigger_summary`,
/// `message`, and routing `tags` into the system prompt so the executor LLM
/// knows exactly what to do and how to publish its completion report.
pub fn scheduled_job_manifest(
    job_id: &str,
    trigger_summary: &str,
    message: &str,
    tags: &[String],
) -> AgentManifest {
    let tags_str = if tags.is_empty() {
        String::new()
    } else {
        format!("\nRouting tags: {}\n", tags.join(", "))
    };

    let system_prompt = format!(
        "You are a scheduled task executor.\n\n## Task\nJob ID: {job_id}\nSchedule: \
         {trigger_summary}\nTask: {message}{tags_str}\n\n## Instructions\n1. Execute the task \
         described above using available tools.\n2. After completion, provide a brief summary of \
         what you did and the outcome.\n\n## After Completion\nWhen you finish the task, call the \
         `kernel` tool with:\n- action: \"publish_report\"\n- report: {{ \"task_id\": \"<uuid>\", \
         \"task_type\": \"<type>\", \"tags\": [<routing tags>], \"status\": \"completed\", \
         \"summary\": \"<one-line summary>\", \"result\": {{<structured result>}} \
         }}\n\nAlternatively, use action: \"publish\" with event_type: \"scheduled_task_done\" \
         and payload: {{ \"message\": \"<summary>\" }}\n"
    );

    AgentManifest {
        name: "scheduled_job".to_string(),
        role: AgentRole::Worker,
        description: "Executes a scheduled task and summarizes the result".to_string(),
        model: None,
        system_prompt,
        soul_prompt: None,
        provider_hint: None,
        max_iterations: Some(15),
        tools: vec![],
        excluded_tools: vec![],
        max_children: Some(0),
        max_context_tokens: None,
        priority: Priority::default(),
        metadata: serde_json::json!({
            "scheduled_job_id": job_id,
        }),
        sandbox: None,
        default_execution_mode: None,
        tool_call_limit: None,
        worker_timeout_secs: None,
        max_continuations: Some(0),
    }
}
