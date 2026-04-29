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
//! `SCHEMA_VERSION: 0.2.0`. Adding attributes is a **minor** change. Renaming
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
//!   Layer A flags a regression. Layer B payloads are written under the
//!   `langfuse.*.input` / `langfuse.*.output` keys so Langfuse renders them in
//!   its UI directly.
//! - **Layer C — pointers, never embedded payloads**: e.g. [`RARA_LOG_FILE`]
//!   points to a hourly log file rather than embedding the lines.
//!
//! Upstream OTel GenAI semantic conventions are imported via
//! `opentelemetry_semantic_conventions::attribute::*` — never hardcode the
//! string form of those keys. Langfuse-specific keys (`langfuse.*`) are not
//! in the OTel semconv registry and are hardcoded here.

/// The semantic-convention version exported by this module. Bumped when
/// attributes are renamed or removed. Detectors pin against this value.
pub const SCHEMA_VERSION: &str = "0.2.0";

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
// langfuse.* — Langfuse-recognized attributes
//
// Langfuse maps these directly into its UI's session/user/environment filters
// and the trace/observation Input/Output panels. They are NOT in the OTel
// semconv registry; the canonical reference is the Langfuse OTel docs:
// https://langfuse.com/docs/opentelemetry/get-started
// ---------------------------------------------------------------------------

/// Langfuse session id — Langfuse groups traces sharing this value into a
/// single session view.
pub const LANGFUSE_SESSION_ID: &str = "langfuse.session.id";

/// Langfuse user id — surfaces in the per-user breakdown.
pub const LANGFUSE_USER_ID: &str = "langfuse.user.id";

/// Trace-level input (typically the user's prompt for the turn). Rendered in
/// the Langfuse traces list "Input" column.
pub const LANGFUSE_TRACE_INPUT: &str = "langfuse.trace.input";

/// Trace-level output (typically the final assistant reply). Rendered in the
/// Langfuse traces list "Output" column.
pub const LANGFUSE_TRACE_OUTPUT: &str = "langfuse.trace.output";

/// Deployment environment label (e.g. `"dev"`, `"prod"`). Lets Langfuse
/// filter / aggregate by environment.
pub const LANGFUSE_ENVIRONMENT: &str = "langfuse.environment";

/// Observation type. Langfuse-recognized values: `"generation"` for LLM
/// calls, `"span"` for everything else (tool execution, root agent turn).
pub const LANGFUSE_OBSERVATION_TYPE: &str = "langfuse.observation.type";

/// Observation type value: an LLM generation. Maps to Langfuse's
/// `GENERATION` observation, which expects model + usage + input + output.
pub const OBSERVATION_TYPE_GENERATION: &str = "generation";

/// Observation type value: a generic span (root turn, tool execution, etc.).
pub const OBSERVATION_TYPE_SPAN: &str = "span";

/// Per-observation input payload. For LLM observations, the serialized
/// request messages; for tool observations, the call arguments.
pub const LANGFUSE_OBSERVATION_INPUT: &str = "langfuse.observation.input";

/// Per-observation output payload. For LLM observations, the assistant
/// response; for tool observations, the tool result JSON.
pub const LANGFUSE_OBSERVATION_OUTPUT: &str = "langfuse.observation.output";

/// Observation severity level. Langfuse-recognized values include
/// `DEBUG`, `DEFAULT`, `WARNING`, `ERROR`. Rara only sets this to
/// [`OBSERVATION_LEVEL_ERROR`] on tool-failure spans; absence implies
/// `DEFAULT`.
pub const LANGFUSE_OBSERVATION_LEVEL: &str = "langfuse.observation.level";

/// Observation level value used on errored tool / generation observations.
pub const OBSERVATION_LEVEL_ERROR: &str = "ERROR";

/// JSON-serialized model parameters (temperature, max_tokens, top_p, …)
/// attached to LLM generation observations.
pub const LANGFUSE_OBSERVATION_MODEL_PARAMETERS: &str = "langfuse.observation.model.parameters";

/// JSON-serialized token usage details (e.g.
/// `{"input": 123, "output": 456}`). Optional — Langfuse already reads
/// `gen_ai.usage.*` for primary token counts.
pub const LANGFUSE_OBSERVATION_USAGE_DETAILS: &str = "langfuse.observation.usage_details";

/// JSON-serialized cost details. Optional — populated only when the
/// provider returns explicit USD costs.
pub const LANGFUSE_OBSERVATION_COST_DETAILS: &str = "langfuse.observation.cost_details";

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

    /// Langfuse keys are hardcoded — Langfuse does not publish a semconv
    /// crate. Renaming any of these silently empties the Langfuse UI panels,
    /// which is exactly what #2002 fixed; pin the strings.
    #[test]
    fn langfuse_keys_have_stable_strings() {
        assert_eq!(LANGFUSE_SESSION_ID, "langfuse.session.id");
        assert_eq!(LANGFUSE_USER_ID, "langfuse.user.id");
        assert_eq!(LANGFUSE_TRACE_INPUT, "langfuse.trace.input");
        assert_eq!(LANGFUSE_TRACE_OUTPUT, "langfuse.trace.output");
        assert_eq!(LANGFUSE_ENVIRONMENT, "langfuse.environment");
        assert_eq!(LANGFUSE_OBSERVATION_TYPE, "langfuse.observation.type");
        assert_eq!(OBSERVATION_TYPE_GENERATION, "generation");
        assert_eq!(OBSERVATION_TYPE_SPAN, "span");
        assert_eq!(LANGFUSE_OBSERVATION_INPUT, "langfuse.observation.input");
        assert_eq!(LANGFUSE_OBSERVATION_OUTPUT, "langfuse.observation.output");
        assert_eq!(LANGFUSE_OBSERVATION_LEVEL, "langfuse.observation.level");
        assert_eq!(OBSERVATION_LEVEL_ERROR, "ERROR");
        assert_eq!(
            LANGFUSE_OBSERVATION_MODEL_PARAMETERS,
            "langfuse.observation.model.parameters"
        );
        assert_eq!(
            LANGFUSE_OBSERVATION_USAGE_DETAILS,
            "langfuse.observation.usage_details"
        );
        assert_eq!(
            LANGFUSE_OBSERVATION_COST_DETAILS,
            "langfuse.observation.cost_details"
        );
    }

    #[test]
    fn schema_version_is_bumped_for_langfuse_renames() {
        // 0.1.0 -> 0.2.0 was the major bump that removed `rara.prompt`,
        // `rara.completion`, `rara.tool.input`, `rara.tool.output` (plus
        // their `*.truncated` siblings) in favour of the `langfuse.*`
        // observation keys. Detectors pin against this value.
        assert_eq!(SCHEMA_VERSION, "0.2.0");
    }
}
