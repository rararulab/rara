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

//! Stable telemetry attribute keys — the contract an external detector reads.
//!
//! `SCHEMA_VERSION: 0.1.0`. Adding attributes is a **minor** change. Renaming
//! or removing one is a **major** change and will break the downstream
//! detector agent — bump the schema version and announce.
//!
//! Three layers, layered by cost and cardinality:
//!
//! - **Layer A — always-on, low-cardinality** (this file): the contract. Set on
//!   every span. The detector decides "is this turn broken?" from these
//!   attributes alone.
//! - **Layer B — content sampling** (see [`crate::payload_sampler`]): set only
//!   when the sampler decides. Bounded by `max_chars`. Used for diagnosis after
//!   Layer A flags a regression.
//! - **Layer C — pointers, never embedded payloads**: e.g. [`RARA_LOG_FILE`]
//!   points to a hourly log file rather than embedding the lines.
//!
//! Upstream OTel GenAI semantic conventions are imported via
//! `opentelemetry_semantic_conventions::attribute::*` — never hardcode the
//! string form of those keys.

/// The semantic-convention version exported by this module. Bumped when
/// attributes are renamed or removed. Detectors pin against this value.
pub const SCHEMA_VERSION: &str = "0.1.0";

// ---------------------------------------------------------------------------
// rara.* — rara-specific attributes
// ---------------------------------------------------------------------------

/// Session id (per `rara_session::SessionKey`) that owns the turn.
///
/// Detector use: group spans into a single conversation; track per-session
/// trends (e.g. context bloat over many turns).
pub const RARA_SESSION_ID: &str = "rara.session.id";

/// Logical agent name (e.g. `"rara"`, `"mita"`) — matches the agent manifest
/// id. NOT the OS process name.
///
/// Detector use: split metrics by agent role; route fix issues to the team
/// owning that agent.
pub const RARA_AGENT_NAME: &str = "rara.agent.name";

/// Skill name (when the turn invoked a skill) — matches the static skill
/// frontmatter `name` field.
///
/// Detector use: detect "skill silent no-op" and route fix to the skill's
/// markdown file.
pub const RARA_SKILL_NAME: &str = "rara.skill.name";

/// Outcome of the turn. One of: `success`, `error`, `aborted`.
///
/// Detector use: top-level success/error split; baseline regression alarm.
pub const RARA_TURN_OUTCOME: &str = "rara.turn.outcome";

/// Iteration index within the agent loop (0-based). Set on per-iteration
/// child spans.
///
/// Detector use: tool-loop runaway detection (`> 10` is suspicious); also
/// "skill silent no-op" detection (iteration=0 + no children).
pub const RARA_TURN_ITERATION: &str = "rara.turn.iteration";

/// Top-level error class on a failed turn. Stable enum-like string.
/// Examples: `llm_rate_limit`, `llm_timeout`, `tool_execution_failed`,
/// `guard_blocked`, `cancelled`, `internal`.
///
/// Detector use: aggregate error rates by class; route by class.
pub const RARA_ERROR_KIND: &str = "rara.error.kind";

/// Tool error sub-class. One of: `timeout`, `panic`, `auth`, `rate_limit`,
/// `invalid_input`, `upstream_error`.
///
/// Detector use: per-tool error decomposition; spot which class is spiking.
pub const RARA_TOOL_ERROR_KIND: &str = "rara.tool.error.kind";

/// Decision returned by the guard pipeline. One of: `allow`, `deny`, `redact`.
///
/// Detector use: deny-rate spike alarms; redaction effectiveness.
pub const RARA_GUARD_DECISION: &str = "rara.guard.decision";

/// Guard rule that fired (matches a `pub const` in
/// [`crate::identifiers`]).
///
/// Detector use: route guard misfires back to the rule definition.
pub const RARA_GUARD_RULE: &str = "rara.guard.rule";

/// Memory subsystem operation. Examples: `tape_append`, `tape_search`,
/// `kv_get`, `kv_set`.
///
/// Detector use: track memory backend health independently of LLM/tool calls.
pub const RARA_MEMORY_OPERATION: &str = "rara.memory.operation";

/// Path to the rotating log file containing lines from this turn —
/// e.g. `/Users/rara/Library/Logs/rara/job.YYYY-MM-DD-HH`. **Layer C
/// pointer**: full text never embedded in the span.
///
/// Detector use: jump from a flagged span straight to the matching log file
/// for raw context.
pub const RARA_LOG_FILE: &str = "rara.log.file";

// ---------------------------------------------------------------------------
// rara.* — Layer B (sampled payload) attributes
// ---------------------------------------------------------------------------

/// Sampled prompt text (Layer B). Truncation is signalled by
/// [`RARA_PROMPT_TRUNCATED`].
pub const RARA_PROMPT: &str = "rara.prompt";
/// True when [`RARA_PROMPT`] was truncated to the sampler's `max_chars`.
pub const RARA_PROMPT_TRUNCATED: &str = "rara.prompt.truncated";

/// Sampled completion text (Layer B). Truncation is signalled by
/// [`RARA_COMPLETION_TRUNCATED`].
pub const RARA_COMPLETION: &str = "rara.completion";
/// True when [`RARA_COMPLETION`] was truncated to the sampler's `max_chars`.
pub const RARA_COMPLETION_TRUNCATED: &str = "rara.completion.truncated";

/// Sampled tool input JSON (Layer B).
pub const RARA_TOOL_INPUT: &str = "rara.tool.input";
/// True when [`RARA_TOOL_INPUT`] was truncated.
pub const RARA_TOOL_INPUT_TRUNCATED: &str = "rara.tool.input.truncated";

/// Sampled tool output JSON (Layer B).
pub const RARA_TOOL_OUTPUT: &str = "rara.tool.output";
/// True when [`RARA_TOOL_OUTPUT`] was truncated.
pub const RARA_TOOL_OUTPUT_TRUNCATED: &str = "rara.tool.output.truncated";

/// Sampled error message chain (Layer B). Always set when sampler decides
/// "on_error".
pub const RARA_ERROR_MESSAGE: &str = "rara.error.message";

// ---------------------------------------------------------------------------
// OpenInference-style span kind attribute
//
// OpenInference is not in the OTel semconv crate; we publish the key here
// so the detector can map every rara span to one of the standard kinds.
// ---------------------------------------------------------------------------

/// OpenInference span kind. One of [`SPAN_KIND_AGENT`], [`SPAN_KIND_LLM`],
/// [`SPAN_KIND_TOOL`], [`SPAN_KIND_RETRIEVER`], [`SPAN_KIND_GUARD`].
pub const OPENINFERENCE_SPAN_KIND: &str = "openinference.span.kind";

/// Root agent turn span kind.
pub const SPAN_KIND_AGENT: &str = "AGENT";
/// LLM call span kind.
pub const SPAN_KIND_LLM: &str = "LLM";
/// Tool execution span kind.
pub const SPAN_KIND_TOOL: &str = "TOOL";
/// Memory/retrieval span kind.
pub const SPAN_KIND_RETRIEVER: &str = "RETRIEVER";
/// Guard pipeline span kind. Not part of the upstream OpenInference enum but
/// used internally so the detector can filter guard spans without parsing the
/// span name.
pub const SPAN_KIND_GUARD: &str = "GUARD";

// ---------------------------------------------------------------------------
// Tool-level attributes (OpenInference-style)
//
// `gen_ai.tool.name` from upstream semconv is also valid and used at the
// LLM-call layer when the model emits a tool call. The keys below describe
// the tool *execution* span itself.
// ---------------------------------------------------------------------------

/// Tool name being executed — value MUST come from
/// [`crate::identifiers`] so renames are caught at compile time.
pub const TOOL_NAME: &str = "tool.name";

/// Tool execution outcome. One of: `success`, `error`.
pub const TOOL_OUTCOME: &str = "tool.outcome";

// ---------------------------------------------------------------------------
// Convenience re-exports of upstream GenAI keys most commonly used on spans.
//
// These are thin pass-throughs so call sites can import from one place,
// but the source of truth is `opentelemetry_semantic_conventions`.
// ---------------------------------------------------------------------------

/// `gen_ai.request.model` — the model id requested from the provider.
pub const GEN_AI_REQUEST_MODEL: &str =
    opentelemetry_semantic_conventions::attribute::GEN_AI_REQUEST_MODEL;

/// `gen_ai.usage.input_tokens` — prompt tokens consumed.
pub const GEN_AI_USAGE_INPUT_TOKENS: &str =
    opentelemetry_semantic_conventions::attribute::GEN_AI_USAGE_INPUT_TOKENS;

/// `gen_ai.usage.output_tokens` — completion tokens produced.
pub const GEN_AI_USAGE_OUTPUT_TOKENS: &str =
    opentelemetry_semantic_conventions::attribute::GEN_AI_USAGE_OUTPUT_TOKENS;

/// `gen_ai.response.finish_reasons` — provider-reported finish reason(s).
pub const GEN_AI_RESPONSE_FINISH_REASONS: &str =
    opentelemetry_semantic_conventions::attribute::GEN_AI_RESPONSE_FINISH_REASONS;

/// `gen_ai.system` — provider system identifier (e.g. `openai`, `anthropic`).
pub const GEN_AI_SYSTEM: &str = opentelemetry_semantic_conventions::attribute::GEN_AI_SYSTEM;

/// `gen_ai.server.time_to_first_token` — TTFT in seconds. Upstream defines
/// this only as a metric name; we reuse the same string for the equivalent
/// span attribute so the detector can join metric and span by key.
pub const GEN_AI_SERVER_TIME_TO_FIRST_TOKEN: &str =
    opentelemetry_semantic_conventions::metric::GEN_AI_SERVER_TIME_TO_FIRST_TOKEN;

#[cfg(test)]
mod tests {
    use super::*;

    /// Stable string values are part of the public contract; the detector
    /// joins on them. Renaming any of these is a major version bump.
    #[test]
    fn rara_keys_have_stable_strings() {
        assert_eq!(RARA_SESSION_ID, "rara.session.id");
        assert_eq!(RARA_TURN_OUTCOME, "rara.turn.outcome");
        assert_eq!(RARA_TOOL_ERROR_KIND, "rara.tool.error.kind");
        assert_eq!(RARA_GUARD_DECISION, "rara.guard.decision");
        assert_eq!(RARA_LOG_FILE, "rara.log.file");
        assert_eq!(OPENINFERENCE_SPAN_KIND, "openinference.span.kind");
        assert_eq!(SPAN_KIND_AGENT, "AGENT");
        assert_eq!(SPAN_KIND_LLM, "LLM");
        assert_eq!(SPAN_KIND_TOOL, "TOOL");
    }

    #[test]
    fn upstream_keys_match_semconv() {
        assert_eq!(GEN_AI_REQUEST_MODEL, "gen_ai.request.model");
        assert_eq!(GEN_AI_USAGE_INPUT_TOKENS, "gen_ai.usage.input_tokens");
        assert_eq!(
            GEN_AI_SERVER_TIME_TO_FIRST_TOKEN,
            "gen_ai.server.time_to_first_token"
        );
    }
}
