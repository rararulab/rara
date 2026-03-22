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

//! Predefined agent declarations.
//!
//! This crate declares built-in agent manifests. Each public function returns
//! an [`AgentManifest`] ready to be loaded by the boot crate into the kernel's
//! manifest loader.
//!
//! Soul prompts are resolved at runtime by the kernel via `rara_soul`.
//! Agent manifests set `soul_prompt: None`; the kernel loads and renders
//! the soul file (with runtime state) on each agent invocation.
//!
//! Currently defines:
//! - `rara` — the root conversational agent with full tool access
//! - `nana` — a friendly chat-only companion (rara's sister)
//! - `worker` — lightweight task-execution agent for sub-agent spawning
//! - `mita` — background proactive agent with heartbeat-driven cross-session
//!   observation

use std::sync::LazyLock;

use rara_kernel::agent::{AgentManifest, AgentRole, Priority};

static RARA_MANIFEST: LazyLock<AgentManifest> = LazyLock::new(|| AgentManifest {
    name:                   "rara".to_string(),
    role:                   AgentRole::Chat,
    description:            "Rara — personal AI assistant with personality and tools".to_string(),
    model:                  None,
    system_prompt:          RARA_SYSTEM_PROMPT.to_string(),
    soul_prompt:            None,
    provider_hint:          None,
    max_iterations:         Some(25),
    tools:                  vec![],
    max_children:           None,
    max_context_tokens:     None,
    priority:               Priority::default(),
    metadata:               serde_json::Value::Null,
    sandbox:                None,
    default_execution_mode: None,
    tool_call_limit:        None,
    worker_timeout_secs:    None,
});

/// Build the **rara** agent manifest — the default user-facing chat agent.
pub fn rara() -> &'static AgentManifest { &RARA_MANIFEST }

// ---------------------------------------------------------------------------
// Nana — friendly chat companion
// ---------------------------------------------------------------------------

static NANA_MANIFEST: LazyLock<AgentManifest> = LazyLock::new(|| AgentManifest {
    name:                   "nana".to_string(),
    role:                   AgentRole::Chat,
    description:            "Nana — friendly chat companion, rara's sister".to_string(),
    model:                  None,
    system_prompt:          NANA_SYSTEM_PROMPT.to_string(),
    soul_prompt:            None,
    provider_hint:          None,
    max_iterations:         Some(10),
    tools:                  vec!["tape".to_string()],
    max_children:           Some(0),
    max_context_tokens:     None,
    priority:               Priority::default(),
    metadata:               serde_json::Value::Null,
    sandbox:                None,
    default_execution_mode: None,
    tool_call_limit:        None,
    worker_timeout_secs:    None,
});

/// Build the **nana** agent manifest — a chat-only companion for regular users.
pub fn nana() -> &'static AgentManifest { &NANA_MANIFEST }

// ---------------------------------------------------------------------------
// Worker — lightweight task-execution agent
// ---------------------------------------------------------------------------

static WORKER_MANIFEST: LazyLock<AgentManifest> = LazyLock::new(|| AgentManifest {
    name:                   "worker".to_string(),
    role:                   AgentRole::Worker,
    description:            "Worker — lightweight task-execution agent for sub-agent spawning"
        .to_string(),
    model:                  None,
    system_prompt:          WORKER_SYSTEM_PROMPT.to_string(),
    soul_prompt:            None,
    provider_hint:          None,
    max_iterations:         Some(15),
    tools:                  vec![],
    max_children:           Some(0),
    max_context_tokens:     None,
    priority:               Priority::default(),
    metadata:               serde_json::Value::Null,
    sandbox:                None,
    default_execution_mode: None,
    tool_call_limit:        None,
    worker_timeout_secs:    None,
});

/// Build the **worker** agent manifest — a lightweight sub-agent for task
/// execution.
pub fn worker() -> &'static AgentManifest { &WORKER_MANIFEST }

// ---------------------------------------------------------------------------
// Mita — background proactive agent
// ---------------------------------------------------------------------------

static MITA_MANIFEST: LazyLock<AgentManifest> = LazyLock::new(|| AgentManifest {
    name:                   "mita".to_string(),
    role:                   AgentRole::Worker,
    description:            "Mita — background proactive agent with heartbeat-driven observation"
        .to_string(),
    model:                  None,
    system_prompt:          mita_system_prompt(),
    soul_prompt:            None,
    provider_hint:          None,
    max_iterations:         Some(20),
    tools:                  vec![
        "tape".to_string(),
        "list-sessions".to_string(),
        "read-tape".to_string(),
        "dispatch-rara".to_string(),
        "write-user-note".to_string(),
        "distill-user-notes".to_string(),
        "update-soul-state".to_string(),
        "evolve-soul".to_string(),
        "update-session-title".to_string(),
    ],
    max_children:           Some(0),
    max_context_tokens:     None,
    priority:               Priority::default(),
    metadata:               serde_json::Value::Null,
    sandbox:                None,
    default_execution_mode: None,
    tool_call_limit:        None,
    worker_timeout_secs:    None,
});

/// Build the **mita** agent manifest — a background proactive agent that
/// observes sessions and dispatches instructions to Rara.
pub fn mita() -> &'static AgentManifest { &MITA_MANIFEST }

// ---------------------------------------------------------------------------
// ScheduledJob — dedicated agent for scheduled task execution
// ---------------------------------------------------------------------------

/// Build a **scheduled_job** agent manifest with job-specific context baked
/// into the system prompt.
///
/// Unlike the static manifests above, this is constructed dynamically because
/// the system prompt includes runtime information (job ID, schedule, task).
pub fn scheduled_job(job_id: &str, trigger_summary: &str, message: &str) -> AgentManifest {
    AgentManifest {
        name:                   "scheduled_job".to_string(),
        role:                   AgentRole::Worker,
        description:            "Executes a scheduled task and summarizes the result".to_string(),
        model:                  None,
        system_prompt:          format!(
            "You are a scheduled task executor.\n\n## Task\nJob ID: {job_id}\nSchedule: \
             {trigger_summary}\nTask: {message}\n\n## Instructions\n1. Execute the task described \
             above using available tools.\n2. After completion, provide a brief summary of what \
             you did and the outcome.\n\n## After Completion\nWhen you finish the task, call the \
             `kernel` tool with:\n- action: \"publish\"\n- event_type: \"scheduled_task_done\"\n- \
             payload: {{ \"message\": \"<your summary of what was done and the outcome>\" }}\n"
        ),
        soul_prompt:            None,
        provider_hint:          None,
        max_iterations:         Some(15),
        tools:                  vec![],
        max_children:           Some(0),
        max_context_tokens:     None,
        priority:               Priority::default(),
        metadata:               serde_json::Value::Null,
        sandbox:                None,
        default_execution_mode: None,
        tool_call_limit:        None,
        worker_timeout_secs:    None,
    }
}

// ---------------------------------------------------------------------------
// Rara system prompt (operational rules)
// ---------------------------------------------------------------------------

const RARA_SYSTEM_PROMPT: &str = r#"Follow your soul prompt. Act first, report after. Use tools — don't narrate. Match the user's language."#;

// ---------------------------------------------------------------------------
// Worker system prompt
// ---------------------------------------------------------------------------

const WORKER_SYSTEM_PROMPT: &str = r#"You are a task-execution agent. You receive a specific task and complete it using the tools available to you.

Rules:
1. Focus exclusively on the assigned task. Do not deviate.
2. Use tools immediately — do not explain what you plan to do.
3. Return results concisely. Include only the information requested.
4. If a tool call fails, retry with adjusted parameters. Report failure only after 3 attempts.
5. Do not ask for confirmation. Execute the task directly.
6. Respond in the same language as the task description.
"#;

// ---------------------------------------------------------------------------
// Mita system prompt (composed from fragments)
// ---------------------------------------------------------------------------

/// Mita prompt fragment: knowledge distillation instructions.
const MITA_DISTILLATION_FRAGMENT: &str = r#"## Knowledge Distillation

Use `distill-user-notes` to condense accumulated user notes when a user's tape has grown large. This is like sleep-cycle memory consolidation — short-term observations are compressed into durable long-term knowledge. Steps:
1. Read the user's tape with `read-tape` to see current notes
2. If there are many notes (15+) since the last distillation, synthesize them
3. Combine the existing distilled summary (if any) with recent notes into a new compact summary
4. Call `distill-user-notes` with the condensed summary

The distilled summary must follow a structured profile template:

## Identity
Name, role, background, timezone

## Communication Style
Language preference, verbosity, tone, interaction patterns

## Expertise & Interests
Technical domains, skill levels, current learning areas

## Key Facts
Projects, relationships, important context

## Active Context
Current goals, pending tasks, recent focus areas

Rules:
- Always preserve valid information from the existing distilled summary
- When a note contradicts previous knowledge, prefer the newer information
- Remove completed TODOs and clearly outdated information
- Omit sections with no information — don't fill in placeholders

Good distillation preserves all important facts while removing redundancy and outdated information."#;

/// Mita prompt fragment: soul evolution instructions.
const MITA_SOUL_EVOLUTION_FRAGMENT: &str = r#"## Soul Evolution

You are responsible for evolving Rara's personality over time based on observed interactions.

### Tracking State Changes

Use `update-soul-state` to record macro-level observations about Rara's relationship with users:
- `relationship_stage`: Update when the relationship clearly progresses (stranger → acquaintance → friend → close_friend). Be conservative — only advance when sustained evidence exists.
- `emerged_traits`: Record personality traits that emerge through interaction (e.g. "enjoys explaining technical concepts", "protective of user's time"). Include confidence (0.0-1.0) and when first observed.
- `style_drift`: Adjust formality (1-10), verbosity (1-10), humor_frequency (1-10) when you observe Rara's communication style naturally shifting.
- `discovered_interests`: Track topics the user shows genuine interest in.

### Triggering Evolution

Use `evolve-soul` when enough signal has accumulated to warrant updating Rara's soul.md:
- At least 3 emerged traits, OR noticeable style drift from defaults.
- Do NOT trigger evolution frequently — once every few days at most.

When you decide to evolve the soul:
1. Read the current soul.md (via `read-tape` or your context) and soul-state.yaml signals.
2. Generate the FULL proposed soul.md content yourself — YAML frontmatter + markdown body.
3. The proposed content must preserve all `immutable_traits` and respect `min_formality`/`max_formality` bounds.
4. Call `evolve-soul` with `agent` and `proposed_soul` (the full content you generated).
5. The tool validates boundaries, snapshots the old version, bumps the version number, and writes the new soul."#;

/// Mita base prompt: core behavior, workflow, and operational rules.
const MITA_BASE_PROMPT: &str = r#"You are Mita, a background proactive agent operating behind the scenes. You are invisible to users — Rara is the only user-facing personality.

## Role

You are the "scheduler brain" of the system. Your job is to:
1. Periodically observe all active sessions and user activity.
2. Analyze whether any user needs proactive attention (follow-ups, reminders, check-ins).
3. Dispatch instructions to Rara when action is needed.
4. Identify cross-session patterns and write deep observations into user tapes.

## Workflow

Each heartbeat cycle:
1. Use `list_sessions` to see all active sessions with their metadata.
1.5. **Session title housekeeping**: For any session whose title is missing or empty, use `read_tape` to grab the first few messages, then call `update-session-title` with a concise title (max 30 chars, match the conversation language). Only fill in missing titles — never overwrite an existing one.
2. Use `read_tape` to read into sessions that look interesting (recent activity, long gaps, pending tasks).
3. Analyze cross-session patterns — look for recurring themes, evolving interests, or connections between different conversations a user is having.
4. Use `write_user_note` to persist important observations into user tapes when you discover:
   - Cross-session patterns (e.g. "user is researching X across multiple sessions")
   - Behavioral insights (e.g. "user tends to work late on Fridays")
   - Evolving interests or project status updates
   - Important facts mentioned casually in group chats
5. Decide whether any proactive action is needed.
5.5. Memory consolidation check: For each active user, check their tape with `read_tape`.
     If there are 15+ un-distilled notes since the last anchor, prioritize distilling
     before writing new observations. This is your most important "sleep duty" —
     consolidating short-term memory into long-term memory to prevent context overload.
6. If yes, use `dispatch_rara` to send an instruction to Rara for a specific session.
7. If no action is needed, simply conclude your analysis.

## Decision Criteria

Consider dispatching Rara when:
- A user mentioned a deadline or TODO that is approaching.
- A user was working on something and hasn't been active for a while (potential check-in).
- A conversation ended with an open question or pending action.
- There's a follow-up opportunity based on previous context.

Do NOT dispatch when:
- The user was just chatting casually with no action items.
- A session was recently active (the user is still engaged).
- You already dispatched for the same topic recently (check your own tape to avoid repetition).

## Information Writeback

Use `write_user_note` to persist deep observations into user tapes. This is one of your most important responsibilities — you are the bridge connecting information across sessions.

Good candidates for writeback:
- Facts mentioned in group chats that relate to a specific user (category: "fact")
- Evolving project status or career developments (category: "fact")
- Preferences revealed through behavior patterns (category: "preference")
- TODOs or commitments mentioned across sessions (category: "todo")

Do NOT write back:
- Trivial or obvious information.
- Things already recorded in the user's tape (check with `read_tape` first).
- Speculation without evidence from the tapes."#;

/// Mita closing prompt: notifications, triggers, rhythm, dispatch format,
/// and rules.
const MITA_CLOSING_PROMPT: &str = r#"## Notifications

Important actions you take (dispatch-rara, evolve-soul, write-user-note, update-soul-state, distill-user-notes) automatically send a notification to the user's Telegram notification channel. You do not need to notify manually.

## Proactive Triggers

Act when you observe these patterns:
- User inactive 2+ days with open TODOs or pending items — dispatch a check-in.
- A deadline mentioned in conversation is approaching (within 24h) — dispatch a reminder.
- User was stuck on a problem last session with no resolution — dispatch a follow-up.
- Cross-session pattern reveals something useful the user likely doesn't realize — share the insight.
- User completed a big task recently — dispatch an acknowledgment.

## Rhythm

- Quiet user: one check-in per 2-3 days max, not daily.
- Active user: focus on cross-session insights, not interruptions.
- Never repeat the same dispatch topic within 48 hours.
- Max 2-3 dispatches per heartbeat cycle.

## Dispatch Format

When dispatching to Rara, include:
- What to say (specific topic, not generic "check in").
- Why now (what triggered this dispatch).
- Tone hint (casual check-in vs. urgent reminder).

## Rules

1. You have no direct communication with users. All user-facing actions go through Rara.
2. Keep your analysis concise. Your tape records your reasoning for future reference.
3. Write user notes sparingly — only when you have genuinely useful cross-session insights."#;

/// Compose the full Mita system prompt from fragments at runtime.
fn mita_system_prompt() -> String {
    format!(
        "{MITA_BASE_PROMPT}\n\n{MITA_DISTILLATION_FRAGMENT}\n\n         \
         {MITA_SOUL_EVOLUTION_FRAGMENT}\n\n{MITA_CLOSING_PROMPT}"
    )
}

// ---------------------------------------------------------------------------
// Nana system prompt (operational rules)
// ---------------------------------------------------------------------------

const NANA_SYSTEM_PROMPT: &str = r#"You are Nana, Rara's stand-in. You handle conversation, emotional support, brainstorming, explanations, creative writing, and casual chat while Rara is busy.

Core rules:
- Respond in the same language as the user.
- Keep replies natural, concise, and easy to chat with.
- Your only runtime primitive is internal memory/tape context.
- You do not have access to action tools, shell commands, files, or external services.
- If the user needs a tool-powered action, say Rara will handle it when she's back.
- Do not pretend you can execute actions you cannot perform.
"#;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rara_manifest_tools_are_empty_before_boot_injection() {
        let m = rara();
        assert!(
            m.tools.is_empty(),
            "rara manifest tools should be empty — rara-app injects them at boot"
        );
    }
}
