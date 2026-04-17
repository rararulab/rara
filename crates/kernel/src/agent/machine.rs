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

//! Pure (sans-IO) agent turn-loop state machine.
//!
//! [`AgentMachine`] models the high-level spine of `agent::run_agent_loop`:
//!
//! ```text
//!   TurnStarted ──▶ AwaitingLlm ──┬─▶ LlmCompleted{tool_calls=∅} ──▶ Finish(Stopped)
//!                                 ├─▶ LlmCompleted{tool_calls=≠∅} ──▶ ExecutingTools
//!                                 │                                   │
//!                                 │                          ToolsCompleted
//!                                 │                                   │
//!                                 │                                   ▼
//!                                 │                              AwaitingLlm
//!                                 ├─▶ LlmFailed{retryable=true,recoveries<MAX} ──▶ AwaitingLlm
//!                                 ├─▶ LlmFailed{retryable=false}             ──▶ Fail
//!                                 ├─▶ GuardRejected                          ──▶ Fail
//!                                 └─▶ Interrupted                            ──▶ Fail
//! ```
//!
//! The machine is **pure**: every transition is a synchronous function from
//! `(state, event)` to `(new state, Vec<Effect>)`. No I/O, no `.await`, no
//! globals.  This is what makes the unit tests at the bottom of this file
//! possible without mocking five subsystems.
//!
//! The runner ([`crate::agent::runner`]) is the async layer that interprets
//! the [`Effect`]s against real subsystems and feeds the outcomes back as
//! [`Event`]s.

use crate::{
    agent::{
        effect::{
            Effect, FinishReason, LimitDecision, PressureLevel, TapeAppendKind, ToolCall,
            ToolResult,
        },
        loop_breaker::{LoopBreakerConfig, LoopIntervention, ToolCallLoopBreaker},
        mood,
        repetition::RepetitionGuard,
    },
    cascade::CascadeAssembler,
    tool::ToolName,
};

/// Usage fraction at which the machine emits a `Warning`-level
/// `ContextPressureWarning` effect. Mirrors the legacy
/// `CONTEXT_WARN_THRESHOLD` constant in `agent::mod`.
pub const CONTEXT_WARN_THRESHOLD: f64 = 0.70;

/// Per-turn configuration controlling when the machine asks the runner to
/// auto-fold (context compression).
///
/// Mirrors the relevant fields of the kernel-level `ContextFoldingConfig`
/// but is intentionally *decoupled*: the sans-IO machine must stay free of
/// `kernel::KernelConfig` so its unit tests compile without the full
/// subsystem tower. Production wiring copies the two thresholds from the
/// kernel config into this struct when the machine is constructed.
#[derive(Debug, Clone, PartialEq)]
pub struct AutoFoldConfig {
    /// Context-usage ratio at which the observer requests a fold on the
    /// next iteration (typically below the 0.70 warning threshold, e.g.
    /// 0.60). Strictly greater-than comparison preserves the legacy
    /// `pressure > fold_threshold` gate exactly.
    pub fold_threshold:            f64,
    /// Minimum number of tape entries that must have accumulated since the
    /// last successful fold before another fold is allowed. Prevents a
    /// run-away loop in which every iteration folds the same short tail.
    pub min_entries_between_folds: usize,
}

/// Usage fraction at which the machine emits a `Critical`-level
/// `ContextPressureWarning` effect. Mirrors the legacy
/// `CONTEXT_CRITICAL_THRESHOLD` constant in `agent::mod`.
pub const CONTEXT_CRITICAL_THRESHOLD: f64 = 0.85;

/// High-level phases of one agent turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Initial state; no LLM call has been issued yet for this turn.
    Idle,
    /// An LLM streaming call is in flight.
    AwaitingLlm,
    /// LLM responded with tool calls; runner is dispatching them.
    ExecutingTools,
    /// Tool-call-limit tripped; runner is awaiting the user's continue/stop
    /// decision before another LLM call is issued.
    PausedForLimit,
    /// Terminal: turn finished successfully.
    Done,
    /// Terminal: turn failed.
    Failed,
}

/// Maximum number of LLM retry attempts (mirrors the legacy
/// `MAX_LLM_ERROR_RECOVERIES` constant in `agent::mod`).
pub const MAX_LLM_RECOVERIES: u32 = 3;

/// Default maximum self-elected continuations per turn.
pub const DEFAULT_MAX_CONTINUATIONS: usize = 10;

/// Name of the meta-tool the LLM calls to activate deferred tools.
///
/// Appearing (successfully) in a completed tool wave triggers an
/// [`Effect::RefreshDeferredTools`] so the next [`Effect::CallLlm`] sees
/// the newly activated catalog.
pub const DISCOVER_TOOLS_TOOL_NAME: &str = "discover-tools";

/// Mutable state carried across machine transitions for one turn.
///
/// The `bool`-heavy shape is intentional: each flag tracks a distinct
/// one-shot legacy latch (`tools_enabled`, `continuation_pending`,
/// `warned_at_warning`, `warned_at_critical`, `force_fold_pending`,
/// `fold_disabled`) whose reset semantics differ. Combining them into an
/// enum would require a cross-product state explosion without any safety
/// or clarity gain.
#[derive(Debug)]
#[allow(clippy::struct_excessive_bools)]
pub struct AgentMachine {
    phase:                Phase,
    iteration:            usize,
    max_iterations:       usize,
    tool_calls_made:      usize,
    last_assistant_text:  String,
    tools_enabled:        bool,
    llm_recoveries:       u32,
    /// Whether the most recent tool wave included a continue-work signal.
    continuation_pending: bool,
    /// How many self-elected continuations have been consumed this turn.
    continuation_count:   usize,
    /// Maximum allowed continuations per turn.
    max_continuations:    usize,
    /// Tool-call-loop detector. Fingerprints every tool call and, when
    /// patterns (exact duplicates, ping-pong, flooding) are detected,
    /// returns the set of tools to disable for the remainder of the turn.
    loop_breaker:         ToolCallLoopBreaker,
    /// Tools the loop breaker has disabled, accumulated across the turn.
    /// Threaded into every subsequent [`Effect::CallLlm`].
    disabled_tools:       Vec<ToolName>,
    /// Tool-call-limit circuit breaker: pause the turn every this many
    /// cumulative tool calls and ask the user whether to continue. Zero
    /// disables the circuit breaker entirely.
    limit_interval:       usize,
    /// Next `tool_calls_made` threshold at which to pause.
    next_limit_at:        usize,
    /// Monotonic counter for limit-pause ids, handed to the runner so it
    /// can key its oneshot channel and reject stale decisions.
    limit_id_counter:     u64,
    /// Whether a `PressureLevel::Warning` has already been emitted this
    /// turn.  The legacy loop nudged the LLM once at each threshold
    /// crossing to avoid spamming a repeating reminder every iteration.
    warned_at_warning:    bool,
    /// Whether a `PressureLevel::Critical` has already been emitted this
    /// turn.  Critical can fire even after Warning has fired (they are
    /// distinct thresholds), but each is still one-shot.
    warned_at_critical:   bool,
    /// Streaming repetition detector. Reset at the start of every LLM
    /// round (fresh accumulated buffer per iteration) so only *intra*-round
    /// verbatim loops trip it. The legacy loop fingerprints each `TextDelta`
    /// against the per-iteration `accumulated_text`; mirror that contract
    /// exactly by wiping state the instant the machine enters
    /// [`Phase::AwaitingLlm`].
    repetition_guard:     RepetitionGuard,
    /// Auto-fold configuration. `None` means auto-fold is disabled for this
    /// turn: neither the observer nor [`AgentMachine::request_force_fold`]
    /// will flip `force_fold_pending`. The recovery branches of
    /// [`AgentMachine::handle_llm_failed`] still request folds regardless
    /// (matching the legacy loop, where the empty-stream and rate-limit
    /// paths set `force_fold_next_iteration` even when auto-fold is off —
    /// the runner is responsible for no-oping the request when the subsystem
    /// cannot actually fold).
    auto_fold_config:     Option<AutoFoldConfig>,
    /// Whether the next [`AgentMachine::rebuild_then_call`] should emit a
    /// leading [`Effect::ForceFoldNextIteration`]. The flag is set by the
    /// observer (pressure-driven), by [`AgentMachine::request_force_fold`]
    /// (for `ToolHint::SuggestFold`), and by the recovery paths. It is
    /// cleared the next time `rebuild_then_call` fires — exactly mirroring
    /// the legacy `force_fold_next_iteration = false` reset inside the
    /// iteration preamble.
    force_fold_pending:   bool,
    /// Set once a fold attempt has failed in this turn; subsequent
    /// auto-trigger calls (`observe_fold_pressure`, `request_force_fold`)
    /// become no-ops so the turn does not hammer a provider that already
    /// rejected the summarization call. Mirrors the legacy
    /// `fold_failed_this_turn` latch. Recovery paths are NOT gated by this
    /// flag — they still emit [`Effect::ForceFoldNextIteration`] and rely
    /// on the subsystem to no-op when its own fold path has been disabled.
    fold_disabled:        bool,
    /// Real-time cascade trace assembler. Entries are appended via
    /// [`AgentMachine::observe_user_input`] (turn start) and implicitly on
    /// every `LlmCompleted` / `ToolsCompleted` transition. The finished
    /// trace is consumed once, via [`AgentMachine::finalize_cascade_trace`],
    /// when the machine first produces a terminal [`Effect::Finish`] /
    /// [`Effect::Fail`].
    cascade_asm:          CascadeAssembler,
    /// Latched once [`Effect::EmitCascadeTrace`] has been prepended to a
    /// terminal effect vector so a double-terminal step (e.g. the fallback
    /// `invalid transition` arm firing after `Done`) does not emit a
    /// duplicate trace.
    cascade_emitted:      bool,
    /// Every non-empty assistant text seen this turn, appended in order
    /// (intermediate + final). Fed into [`mood::infer_mood`] at
    /// end-of-turn; the inference itself only inspects the tail window.
    assistant_texts:      Vec<String>,
}

impl AgentMachine {
    /// Construct a fresh machine with the configured iteration ceiling.
    pub fn new(max_iterations: usize) -> Self {
        Self {
            phase: Phase::Idle,
            iteration: 0,
            max_iterations,
            tool_calls_made: 0,
            last_assistant_text: String::new(),
            tools_enabled: true,
            llm_recoveries: 0,
            continuation_pending: false,
            continuation_count: 0,
            max_continuations: DEFAULT_MAX_CONTINUATIONS,
            loop_breaker: ToolCallLoopBreaker::new(LoopBreakerConfig::builder().build()),
            disabled_tools: Vec::new(),
            limit_interval: 0,
            next_limit_at: usize::MAX,
            limit_id_counter: 0,
            warned_at_warning: false,
            warned_at_critical: false,
            repetition_guard: RepetitionGuard::new(),
            auto_fold_config: None,
            force_fold_pending: false,
            fold_disabled: false,
            cascade_asm: CascadeAssembler::new(String::new()),
            cascade_emitted: false,
            assistant_texts: Vec::new(),
        }
    }

    /// Construct a machine with an auto-fold configuration.
    ///
    /// When `cfg` is present, [`AgentMachine::observe_fold_pressure`] and
    /// [`AgentMachine::request_force_fold`] become operational: they flip
    /// `force_fold_pending`, which on the next pre-LLM boundary prepends an
    /// [`Effect::ForceFoldNextIteration`] to the emitted effect list. When
    /// `cfg` is `None` (the default), only the LLM-failure recovery paths
    /// emit fold requests — matching the legacy loop with
    /// `context_folding.enabled = false`.
    pub(crate) fn with_auto_fold(max_iterations: usize, cfg: AutoFoldConfig) -> Self {
        Self {
            auto_fold_config: Some(cfg),
            ..Self::new(max_iterations)
        }
    }

    /// Construct a machine with a tool-call-limit circuit breaker.
    ///
    /// The turn will pause every `limit_interval` cumulative tool calls
    /// and emit [`Effect::PauseForLimit`]; the runner awaits the user's
    /// decision and feeds it back via [`Event::LimitResolved`]. A
    /// `limit_interval` of zero disables the circuit breaker.
    pub(crate) fn with_tool_call_limit(max_iterations: usize, limit_interval: usize) -> Self {
        Self {
            limit_interval,
            next_limit_at: if limit_interval == 0 {
                usize::MAX
            } else {
                limit_interval
            },
            ..Self::new(max_iterations)
        }
    }

    /// Construct a machine with custom iteration and continuation limits.
    pub fn with_max_continuations(max_iterations: usize, max_continuations: usize) -> Self {
        Self {
            max_continuations,
            ..Self::new(max_iterations)
        }
    }

    /// Construct a machine with a custom [`LoopBreakerConfig`].
    ///
    /// Callers use this to pass a `flooding_exempt` set (e.g. the current
    /// turn's read-only tools) so the breaker does not disable them on long
    /// investigations with many distinct arguments — mirroring the
    /// `t.is_read_only(...)` exemption the legacy `run_agent_loop` builds.
    pub(crate) fn with_loop_breaker_config(
        max_iterations: usize,
        loop_breaker: LoopBreakerConfig,
    ) -> Self {
        Self {
            loop_breaker: ToolCallLoopBreaker::new(loop_breaker),
            ..Self::new(max_iterations)
        }
    }

    /// Current high-level phase.
    pub fn phase(&self) -> Phase { self.phase }

    /// Iteration counter (0-based).
    pub fn iteration(&self) -> usize { self.iteration }

    /// Cumulative tool calls executed in the turn so far.
    pub fn tool_calls_made(&self) -> usize { self.tool_calls_made }

    /// How many self-elected continuations have been consumed this turn.
    pub fn continuation_count(&self) -> usize { self.continuation_count }

    /// Whether the machine has reached a terminal state.
    pub fn is_terminal(&self) -> bool { matches!(self.phase, Phase::Done | Phase::Failed) }

    /// Synchronous observation: report the current context-window usage and
    /// return any resulting `ContextPressureWarning` effects.
    ///
    /// The runner is expected to call this once per LLM round — after
    /// rebuilding the tape context for the next iteration but before
    /// interpreting `Effect::CallLlm` — so the injected warning message lands
    /// in the conversation immediately ahead of the model's next turn.
    ///
    /// Unlike `step`, this method does **not** consume an [`Event`] and does
    /// **not** change [`Phase`]. It is a pure observation:
    ///
    /// - Returns `[Effect::ContextPressureWarning { level: Critical, .. }]` the
    ///   first time usage crosses `CONTEXT_CRITICAL_THRESHOLD`.
    /// - Returns `[Effect::ContextPressureWarning { level: Warning, .. }]` the
    ///   first time usage crosses `CONTEXT_WARN_THRESHOLD` (and Critical has
    ///   not yet fired).
    /// - Returns `[]` otherwise (already warned at the bucket, or below
    ///   threshold, or window is zero).
    ///
    /// Warning and Critical each fire at most once per turn — the legacy
    /// loop nudged the LLM once per crossing to avoid spam that would anchor
    /// the conversation on the reminder itself.
    pub fn observe_context_usage(
        &mut self,
        estimated_tokens: usize,
        context_window_tokens: usize,
    ) -> Vec<Effect> {
        if context_window_tokens == 0 {
            return Vec::new();
        }
        let usage_ratio = estimated_tokens as f64 / context_window_tokens as f64;

        if usage_ratio >= CONTEXT_CRITICAL_THRESHOLD && !self.warned_at_critical {
            self.warned_at_critical = true;
            // Ensure Warning is also marked as delivered once we jumped
            // straight past it — avoids emitting a stale Warning after a
            // Critical has already been surfaced.
            self.warned_at_warning = true;
            return vec![Effect::ContextPressureWarning {
                level: PressureLevel::Critical,
                estimated_tokens,
                context_window_tokens,
            }];
        }

        if usage_ratio >= CONTEXT_WARN_THRESHOLD && !self.warned_at_warning {
            self.warned_at_warning = true;
            return vec![Effect::ContextPressureWarning {
                level: PressureLevel::Warning,
                estimated_tokens,
                context_window_tokens,
            }];
        }

        Vec::new()
    }

    /// Synchronous observation: evaluate the legacy auto-fold trigger
    /// (`pressure > fold_threshold` AND `entries_since_last_fold >=
    /// min_entries_between_folds`) and, if satisfied, flip the internal
    /// `force_fold_pending` flag so the next pre-LLM boundary prepends an
    /// [`Effect::ForceFoldNextIteration`].
    ///
    /// Returns `true` if a fold was just requested (the caller may log or
    /// emit a metric), `false` otherwise. Calling on a machine without an
    /// [`AutoFoldConfig`] always returns `false` — auto-fold is disabled.
    ///
    /// This observer does NOT emit effects directly: the legacy loop runs
    /// the fold *at the top of the next iteration*, after recovery-injected
    /// messages have been written, so the flag-then-emit-on-rebuild shape
    /// preserves the legacy ordering
    /// `[ForceFoldNextIteration?, Inject*?, RebuildTape, CallLlm]` (with
    /// `Inject*` interleaved by whichever path triggered it).
    ///
    /// Arguments mirror the legacy call site:
    ///
    /// - `estimated_tokens` / `context_window_tokens`: used to compute the
    ///   usage ratio the same way as [`AgentMachine::observe_context_usage`].
    /// - `entries_since_last_fold`: count of tape entries since the most recent
    ///   successful auto-fold anchor; the runner reads this from the tape (or
    ///   supplies the total entry count when no fold has run yet).
    ///
    /// Subsequent calls after [`AgentMachine::mark_fold_failed`] become
    /// no-ops — the machine will not request further folds in this turn.
    pub fn observe_fold_pressure(
        &mut self,
        estimated_tokens: usize,
        context_window_tokens: usize,
        entries_since_last_fold: usize,
    ) -> bool {
        let Some(cfg) = self.auto_fold_config.as_ref() else {
            return false;
        };
        if self.fold_disabled || context_window_tokens == 0 {
            return false;
        }
        let ratio = estimated_tokens as f64 / context_window_tokens as f64;
        if ratio > cfg.fold_threshold && entries_since_last_fold >= cfg.min_entries_between_folds {
            self.force_fold_pending = true;
            return true;
        }
        false
    }

    /// Imperative fold request (legacy `ToolHint::SuggestFold`).
    ///
    /// Flips `force_fold_pending` unconditionally *except* when
    /// [`AgentMachine::mark_fold_failed`] has already been called this turn
    /// or the machine has no [`AutoFoldConfig`] attached. Returns whether
    /// the request was honoured.
    ///
    /// The cooldown (`min_entries_between_folds`) is **not** applied here:
    /// the legacy loop's `if should_fold { if force_fold_next_iteration {
    /// … } }` path bypasses the cooldown whenever the flag is set, and
    /// tools that request a fold (e.g. a summariser skill) are trusted to
    /// know the context needs compacting.
    pub fn request_force_fold(&mut self) -> bool {
        if self.auto_fold_config.is_none() || self.fold_disabled {
            return false;
        }
        self.force_fold_pending = true;
        true
    }

    /// Latch that the current turn's fold attempt failed.
    ///
    /// After this call:
    ///
    /// - [`AgentMachine::observe_fold_pressure`] and
    ///   [`AgentMachine::request_force_fold`] become no-ops for the rest of the
    ///   turn — avoids repeatedly hammering a provider that already failed the
    ///   summarisation call.
    /// - Any currently pending fold request is cleared so the next
    ///   `rebuild_then_call` does NOT prepend a stale
    ///   [`Effect::ForceFoldNextIteration`].
    ///
    /// Recovery-path fold requests inside the LLM-failure handler
    /// intentionally bypass this latch (see the rate-limit and empty-stream
    /// branches): legacy `run_agent_loop` keeps emitting fold requests from
    /// those branches and relies on the runtime fold subsystem to no-op.
    pub fn mark_fold_failed(&mut self) {
        self.fold_disabled = true;
        self.force_fold_pending = false;
    }

    /// Whether a fold has been requested but not yet emitted. Primarily
    /// useful for machine-level unit tests; the runner consumes the flag
    /// implicitly by receiving the [`Effect::ForceFoldNextIteration`]
    /// prepended to the next pre-LLM effect pair.
    pub fn force_fold_pending(&self) -> bool { self.force_fold_pending }

    /// Whether further auto-fold requests are latched off for this turn.
    pub fn fold_disabled(&self) -> bool { self.fold_disabled }

    /// Emit the standard pre-LLM effect pair: a [`Effect::RebuildTape`]
    /// immediately followed by a [`Effect::CallLlm`]. Every site that
    /// reaches [`Phase::AwaitingLlm`] uses this so the runner always
    /// regenerates messages from the tape before the call fires —
    /// preserving the legacy `run_agent_loop`'s "tape is the single source
    /// of truth" invariant.
    ///
    /// Takes `&mut self` so it can also reset the per-iteration
    /// [`RepetitionGuard`]: a fresh accumulator starts for every new LLM
    /// round, matching the legacy loop's `let mut accumulated_text =
    /// String::new()` pattern, and clear `force_fold_pending` after the
    /// fold effect is emitted, matching the legacy
    /// `force_fold_next_iteration = false` reset.
    ///
    /// Return type is `Vec<Effect>` (not a fixed-size array) because the
    /// auto-fold path prepends a leading [`Effect::ForceFoldNextIteration`]
    /// when `force_fold_pending` is set, yielding a 3-element slice in that
    /// case. The emit order is
    /// `[ForceFoldNextIteration?, RebuildTape, CallLlm]`; callers that also
    /// need to inject a message interleave it *before* calling
    /// `rebuild_then_call` (see [`AgentMachine::handle_llm_failed`]).
    fn rebuild_then_call(&mut self) -> Vec<Effect> {
        self.repetition_guard = RepetitionGuard::new();
        let mut effects = Vec::with_capacity(3);
        if self.force_fold_pending {
            effects.push(Effect::ForceFoldNextIteration);
            self.force_fold_pending = false;
        }
        effects.push(Effect::RebuildTape {
            iteration: self.iteration,
        });
        effects.push(Effect::CallLlm {
            iteration:      self.iteration,
            tools_enabled:  self.tools_enabled,
            disabled_tools: self.disabled_tools.clone(),
        });
        effects
    }

    /// Synchronous observation: feed a streaming text delta into the
    /// repetition guard and report whether the runner should abort the
    /// in-flight stream.
    ///
    /// The runner calls this between `StreamDelta::TextDelta` events (after
    /// pushing the delta onto its accumulated text buffer). When the guard
    /// detects that the trailing 200 characters of the accumulated output
    /// also appear earlier in the text, it returns a
    /// [`RepetitionAction::Abort`] carrying the byte index at which the
    /// runner should truncate `accumulated_text`. The runner then cancels
    /// the provider stream and synthesises an
    /// [`Event::LlmCompleted`] carrying the truncated text (no tool calls,
    /// empty token usage), matching the legacy loop's
    /// `repetition_aborted = true` branch which skips the driver-error path
    /// entirely.
    ///
    /// Like [`AgentMachine::observe_context_usage`], this method does **not**
    /// consume an [`Event`] and does **not** mutate [`Phase`]. It is a pure
    /// per-delta observation used on a hot path (thousands of calls per
    /// second under streaming), so it intentionally avoids the event/effect
    /// round-trip.
    ///
    /// Arguments match the legacy call site verbatim: `delta` is the new
    /// text just appended, `accumulated` is the full buffer *including* the
    /// delta. The guard tracks its own byte count and will
    /// `debug_assert!(total_bytes == accumulated.len())` — passing the
    /// buffer *before* appending the delta is a caller bug.
    pub fn observe_stream_delta(
        &mut self,
        delta: &str,
        accumulated: &str,
    ) -> Option<RepetitionAction> {
        self.repetition_guard
            .feed(delta, accumulated)
            .map(|truncate_at_byte| RepetitionAction::Abort { truncate_at_byte })
    }

    /// Translate an LLM-failure event into recovery effects.
    ///
    /// Preserves the legacy `run_agent_loop` branching verbatim:
    ///
    /// - [`LlmFailureKind::RateLimited`] with `tool_calls_made > 0`: disable
    ///   tools, inject the "summarize with what you have" nudge, force a fold,
    ///   and retry. Does **not** consume a recovery slot — the legacy path uses
    ///   `continue` without bumping the counter.
    /// - [`LlmFailureKind::RateLimited`] with `tool_calls_made == 0`: falls
    ///   through to the retryable branch (the legacy order tests the rate-limit
    ///   special-case first, then the generic retryable predicate, and 429
    ///   errors satisfy both).
    /// - [`LlmFailureKind::Retryable`]: consumes a recovery slot, disables
    ///   tools, injects the "server error, reply without tools" nudge, retries.
    ///   Fails when the slot budget is exhausted.
    /// - [`LlmFailureKind::EmptyStream`]: consumes a recovery slot, disables
    ///   tools, injects the "empty response, context compressed" nudge, and
    ///   emits [`Effect::ForceFoldNextIteration`] so the runner folds context
    ///   before the next call.
    /// - [`LlmFailureKind::Permanent`]: terminates the turn immediately.
    fn handle_llm_failed(&mut self, kind: LlmFailureKind) -> Vec<Effect> {
        match kind {
            // Rate-limit after at least one tool call: legacy `continue` path
            // — disable tools, inject a final-answer nudge, and force a fold.
            // The legacy code path does NOT increment the recovery counter.
            LlmFailureKind::RateLimited if self.tool_calls_made > 0 => {
                self.tools_enabled = false;
                // Recovery path: request a fold unconditionally so the
                // prepended `ForceFoldNextIteration` in `rebuild_then_call`
                // lands ahead of the follow-up LLM call. Not gated by
                // `fold_disabled` — legacy sets the flag regardless and
                // relies on the runner to no-op when the subsystem has
                // already given up on folds for this turn.
                self.force_fold_pending = true;
                let mut effects = vec![Effect::InjectUserMessage {
                    text: "[System] You hit a rate limit. Do NOT call any more tools. Summarize \
                           the information you already have and answer the user's question now."
                        .to_owned(),
                }];
                effects.extend(self.rebuild_then_call());
                effects
            }

            // Rate-limit with no tool calls made yet falls through to the
            // retryable branch (legacy guard: `rate_limit && tool_calls_made > 0`,
            // else `is_retryable_provider_error` — 429 is classified as both).
            LlmFailureKind::RateLimited => self.retryable_recovery(
                "[System] The previous request encountered a server error (rate limited). Please \
                 reply to the user's question directly without using tools."
                    .to_owned(),
                "rate limited".to_owned(),
            ),

            LlmFailureKind::Retryable { message } => {
                let nudge = format!(
                    "[System] The previous request encountered a server error ({message}). Please \
                     reply to the user's question directly without using tools."
                );
                self.retryable_recovery(nudge, message)
            }

            LlmFailureKind::EmptyStream => {
                if self.llm_recoveries >= MAX_LLM_RECOVERIES {
                    self.phase = Phase::Failed;
                    return vec![Effect::Fail {
                        message: "LLM stream returned empty after max recoveries".to_owned(),
                    }];
                }
                self.llm_recoveries += 1;
                self.tools_enabled = false;
                // Recovery path: see the rate-limit branch comment for why
                // the fold request is unconditional.
                self.force_fold_pending = true;
                let mut effects = vec![Effect::InjectUserMessage {
                    text: "[System] The previous request produced an empty response (possible \
                           context window limit). Context has been compressed. Please reply to \
                           the user's question directly without using tools."
                        .to_owned(),
                }];
                effects.extend(self.rebuild_then_call());
                effects
            }

            LlmFailureKind::Permanent { message } => {
                self.phase = Phase::Failed;
                vec![Effect::Fail { message }]
            }
        }
    }

    /// Shared body for retryable-provider and no-tools rate-limit branches.
    /// Consumes a recovery slot and emits inject+CallLlm; falls back to
    /// `Effect::Fail` when the slot budget is exhausted.
    fn retryable_recovery(&mut self, nudge: String, fail_message: String) -> Vec<Effect> {
        if self.llm_recoveries >= MAX_LLM_RECOVERIES {
            self.phase = Phase::Failed;
            return vec![Effect::Fail {
                message: fail_message,
            }];
        }
        self.llm_recoveries += 1;
        self.tools_enabled = false;
        let mut effects = vec![Effect::InjectUserMessage { text: nudge }];
        effects.extend(self.rebuild_then_call());
        effects
    }

    /// Drive the machine with one event.  Returns the side effects the runner
    /// must perform before feeding the next event back in.
    ///
    /// Push an assistant text sample into the cascade assembler (and the
    /// mood inference corpus). No-op on empty strings to match the legacy
    /// `build_cascade` skip-empty-thought rule and to avoid polluting the
    /// mood window with zero-content responses.
    fn push_cascade_assistant(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.assistant_texts.push(text.to_owned());
        self.cascade_asm
            .push_assistant(text, None, jiff::Timestamp::now(), None);
    }

    /// Record the tool-call wave the LLM just emitted in the cascade trace.
    fn push_cascade_tool_calls(&mut self, calls: &[ToolCall]) {
        if calls.is_empty() {
            return;
        }
        let pairs: Vec<(&str, &str)> = calls
            .iter()
            .map(|c| (c.name.as_str(), c.arguments.as_str()))
            .collect();
        self.cascade_asm
            .push_tool_calls(&pairs, jiff::Timestamp::now(), None);
    }

    /// Record a tool-result wave in the cascade trace. Uses the per-call
    /// `error` string when the tool failed, and a synthesised
    /// "ok ({duration}ms)" marker on success — the machine does not see the
    /// real tool output, only [`ToolResult`] metadata.
    fn push_cascade_tool_results(&mut self, results: &[ToolResult]) {
        if results.is_empty() {
            return;
        }
        let rendered: Vec<String> = results
            .iter()
            .map(|r| match (&r.error, r.success) {
                (Some(msg), _) => msg.clone(),
                (None, true) => format!("ok ({}ms)", r.duration_ms),
                (None, false) => "error".to_owned(),
            })
            .collect();
        let refs: Vec<&str> = rendered.iter().map(String::as_str).collect();
        self.cascade_asm
            .push_tool_results(&refs, jiff::Timestamp::now(), None);
    }

    /// Consume the assembled trace + compute mood inference, latching
    /// `cascade_emitted` so a duplicate terminal step does not re-emit the
    /// effect. Returns `None` on the second and subsequent calls — the
    /// caller should only invoke this when prepending
    /// [`Effect::EmitCascadeTrace`] to a terminal effect vector.
    fn finalize_cascade_trace(&mut self) -> Option<Effect> {
        if self.cascade_emitted {
            return None;
        }
        self.cascade_emitted = true;
        // Replace with a throwaway empty assembler; the original holds the
        // accumulated entries and can be consumed by `finish`.
        let asm = std::mem::replace(&mut self.cascade_asm, CascadeAssembler::new(String::new()));
        let trace = asm.finish();
        let mood = mood::infer_mood(&self.assistant_texts);
        Some(Effect::EmitCascadeTrace { trace, mood })
    }

    /// If `effects` contains a terminal [`Effect::Finish`] or
    /// [`Effect::Fail`], prepend a single [`Effect::EmitCascadeTrace`]
    /// immediately before the first terminal effect. Idempotent: further
    /// calls are no-ops once `cascade_emitted` latches.
    ///
    /// Keeping the injection central here — rather than modifying every
    /// `Effect::Finish` / `Effect::Fail` construction site — keeps the
    /// terminal paths syntactically unchanged and guarantees every exit
    /// carries the same trace+mood payload in the same relative position.
    fn inject_cascade_trace(&mut self, mut effects: Vec<Effect>) -> Vec<Effect> {
        if self.cascade_emitted {
            return effects;
        }
        let terminal_idx = effects
            .iter()
            .position(|e| matches!(e, Effect::Finish { .. } | Effect::Fail { .. }));
        if let Some(idx) = terminal_idx
            && let Some(cascade_effect) = self.finalize_cascade_trace()
        {
            effects.insert(idx, cascade_effect);
        }
        effects
    }

    /// Seed the cascade trace with the user input that started this turn.
    ///
    /// Must be called once, before driving the machine with
    /// [`Event::TurnStarted`], so the assembled trace's first tick contains a
    /// [`CascadeEntryKind::UserInput`] entry. The runner typically calls this
    /// together with [`AgentMachine::set_cascade_message_id`] immediately
    /// after constructing the machine.
    ///
    /// [`CascadeEntryKind::UserInput`]: crate::cascade::CascadeEntryKind::UserInput
    pub fn observe_user_input(&mut self, text: &str) {
        self.cascade_asm
            .push_user(text, jiff::Timestamp::now(), None);
    }

    /// Set the cascade trace's message id (typically a Rara-side message
    /// handle). Call this exactly once, before [`Event::TurnStarted`].
    ///
    /// Left as a setter rather than a constructor argument because adding a
    /// mandatory field to every existing constructor would ripple through
    /// every call site and every test; the assembler tolerates an empty id
    /// and the runner only reads the id when it serialises the trace for
    /// downstream persistence.
    pub fn set_cascade_message_id(&mut self, id: String) {
        let previous = std::mem::replace(&mut self.cascade_asm, CascadeAssembler::new(id));
        let drained = previous.finish();
        // Late calls (after entries were pushed) silently drop the prior
        // partial trace — a loud warning surfaces the misuse without
        // corrupting the new turn's trace.
        if !drained.ticks.is_empty() {
            tracing::warn!(
                "set_cascade_message_id called after cascade entries were already pushed; \
                 dropping prior entries"
            );
        }
    }

    /// Calling `step` after the machine has reached [`Phase::Done`] or
    /// [`Phase::Failed`] is a logic error and produces no effects.
    pub fn step(&mut self, event: Event) -> Vec<Effect> {
        // Real-time cascade assembly: record the entries implied by the
        // incoming event before dispatching the transition so the trace
        // mirrors what the legacy `run_agent_loop` publishes. The push
        // helpers are no-ops on empty payloads; we inspect the event by
        // reference first so the borrow checker accepts the later `match`
        // that consumes it.
        match &event {
            Event::LlmCompleted {
                text, tool_calls, ..
            } => {
                self.push_cascade_assistant(text);
                self.push_cascade_tool_calls(tool_calls);
            }
            Event::ToolsCompleted { results } => {
                self.push_cascade_tool_results(results);
            }
            _ => {}
        }

        let effects = self.step_inner(event);
        self.inject_cascade_trace(effects)
    }

    /// Pure transition function: consumes the event and returns the raw
    /// effect list without cascade-trace injection. Split out so the outer
    /// [`AgentMachine::step`] can centralise `EmitCascadeTrace` emission
    /// without the transition arms needing to know about it.
    fn step_inner(&mut self, event: Event) -> Vec<Effect> {
        match (self.phase, event) {
            // ── Turn boot ────────────────────────────────────────────────
            (Phase::Idle, Event::TurnStarted) => {
                self.phase = Phase::AwaitingLlm;
                self.rebuild_then_call()
            }

            // ── LLM produced a terminal response ─────────────────────────
            (
                Phase::AwaitingLlm,
                Event::LlmCompleted {
                    text,
                    tool_calls,
                    has_tool_calls,
                },
            ) if !has_tool_calls => {
                self.last_assistant_text = text.clone();
                debug_assert!(tool_calls.is_empty());

                // If a previous tool wave signaled continue-work AND budget remains,
                // loop back instead of stopping.
                if self.continuation_pending && self.continuation_count < self.max_continuations {
                    self.continuation_pending = false;
                    self.continuation_count += 1;
                    self.phase = Phase::AwaitingLlm;
                    let mut effects = vec![
                        Effect::AppendTape {
                            kind: TapeAppendKind::AssistantIntermediate,
                        },
                        Effect::InjectContinuationWake {
                            turn: self.continuation_count,
                            max:  self.max_continuations,
                        },
                    ];
                    effects.extend(self.rebuild_then_call());
                    return effects;
                }

                self.continuation_pending = false;
                self.phase = Phase::Done;
                vec![
                    Effect::AppendTape {
                        kind: TapeAppendKind::AssistantFinal,
                    },
                    Effect::Finish {
                        text,
                        iterations: self.iteration + 1,
                        tool_calls: self.tool_calls_made,
                        reason: FinishReason::Stopped,
                    },
                ]
            }

            // ── LLM produced tool calls ──────────────────────────────────
            (
                Phase::AwaitingLlm,
                Event::LlmCompleted {
                    text,
                    tool_calls,
                    has_tool_calls: true,
                },
            ) => {
                self.last_assistant_text = text;
                self.tool_calls_made += tool_calls.len();
                self.phase = Phase::ExecutingTools;
                vec![
                    Effect::AppendTape {
                        kind: TapeAppendKind::AssistantIntermediate,
                    },
                    Effect::AppendTape {
                        kind: TapeAppendKind::ToolCalls,
                    },
                    Effect::RunTools { calls: tool_calls },
                ]
            }

            // ── LLM error: branch on failure kind ────────────────────────
            (Phase::AwaitingLlm, Event::LlmFailed { kind }) => self.handle_llm_failed(kind),

            // ── Tool wave finished ───────────────────────────────────────
            (Phase::ExecutingTools, Event::ToolsCompleted { results }) => {
                // Check if any tool in this wave signaled continuation.
                self.continuation_pending = results
                    .iter()
                    .any(|r| r.name == "continue-work" && r.success);

                // Feed every call from this wave into the loop breaker, then
                // consult it exactly once.  Fingerprints are (name, arguments)
                // pairs, which `ToolResult.arguments` now preserves.
                for r in &results {
                    self.loop_breaker.record(r.name.as_str(), &r.arguments);
                }
                let loop_breaker_effect = match self.loop_breaker.check() {
                    LoopIntervention::None => None,
                    LoopIntervention::DisableTools { pattern, tools, .. } => {
                        let newly_disabled: Vec<ToolName> = tools
                            .into_iter()
                            .map(ToolName::new)
                            .filter(|t| !self.disabled_tools.contains(t))
                            .collect();
                        self.disabled_tools.extend(newly_disabled.iter().cloned());
                        Some(Effect::LoopBreakerTriggered {
                            disabled_tools:  newly_disabled,
                            pattern:         pattern.to_owned(),
                            tool_calls_made: self.tool_calls_made,
                        })
                    }
                };

                // Collect call ids of successful discover-tools invocations so
                // the runner can resolve their outputs and refresh the LLM
                // tool catalog before the next `CallLlm`.
                let discover_trigger_ids: Vec<_> = results
                    .iter()
                    .filter(|r| r.name == DISCOVER_TOOLS_TOOL_NAME && r.success)
                    .map(|r| r.id.clone())
                    .collect();

                self.iteration += 1;
                let mut effects = vec![Effect::AppendTape {
                    kind: TapeAppendKind::ToolResults,
                }];
                if let Some(e) = loop_breaker_effect {
                    effects.push(e);
                }

                if self.iteration >= self.max_iterations {
                    self.phase = Phase::Done;
                    let text = std::mem::take(&mut self.last_assistant_text);
                    // Terminal wave — no upcoming CallLlm, so the activation
                    // set would never be consulted. Skip the refresh.
                    effects.push(Effect::Finish {
                        text,
                        iterations: self.iteration,
                        tool_calls: self.tool_calls_made,
                        reason: FinishReason::MaxIterations,
                    });
                } else {
                    // Refresh once *before* either the pause or the next LLM
                    // call. When a limit pause follows, the user-resume path
                    // (LimitResolved::Continue) will re-enter `CallLlm` with
                    // the activation set already in place.
                    if !discover_trigger_ids.is_empty() {
                        effects.push(Effect::RefreshDeferredTools {
                            trigger_call_ids: discover_trigger_ids,
                        });
                    }
                    if self.limit_interval > 0 && self.tool_calls_made >= self.next_limit_at {
                        self.limit_id_counter += 1;
                        self.phase = Phase::PausedForLimit;
                        effects.push(Effect::PauseForLimit {
                            limit_id:        self.limit_id_counter,
                            tool_calls_made: self.tool_calls_made,
                        });
                    } else {
                        self.phase = Phase::AwaitingLlm;
                        effects.extend(self.rebuild_then_call());
                    }
                }
                effects
            }

            // ── User decided whether to continue after a limit pause ─────
            (Phase::PausedForLimit, Event::LimitResolved { limit_id, decision })
                if limit_id == self.limit_id_counter =>
            {
                match decision {
                    LimitDecision::Continue => {
                        self.next_limit_at = self.tool_calls_made + self.limit_interval;
                        self.phase = Phase::AwaitingLlm;
                        self.rebuild_then_call()
                    }
                    LimitDecision::Stop => {
                        self.phase = Phase::Done;
                        let text = std::mem::take(&mut self.last_assistant_text);
                        vec![Effect::Finish {
                            text,
                            iterations: self.iteration,
                            tool_calls: self.tool_calls_made,
                            reason: FinishReason::StoppedByLimit,
                        }]
                    }
                }
            }

            // ── Guard rejected the wave ──────────────────────────────────
            (Phase::ExecutingTools, Event::GuardRejected { reason }) => {
                self.phase = Phase::Failed;
                vec![Effect::Fail {
                    message: format!("guard rejected tool wave: {reason}"),
                }]
            }

            // ── Interruption from any non-terminal state ─────────────────
            (
                Phase::AwaitingLlm | Phase::ExecutingTools | Phase::PausedForLimit,
                Event::Interrupted,
            ) => {
                self.phase = Phase::Failed;
                vec![Effect::Fail {
                    message: "turn interrupted".to_owned(),
                }]
            }

            // Any other (phase, event) pair is a programming error in the
            // runner — surface it loudly so tests catch contract violations.
            (phase, event) => {
                self.phase = Phase::Failed;
                vec![Effect::Fail {
                    message: format!("invalid transition: phase={phase:?} event={event:?}"),
                }]
            }
        }
    }
}

/// Decision returned by [`AgentMachine::observe_stream_delta`] when the
/// repetition guard fires.
///
/// The runner cancels the provider stream, truncates its accumulated text
/// at `truncate_at_byte`, and synthesises an [`Event::LlmCompleted`] with the
/// truncated text (no tool calls). The legacy loop's `repetition_aborted`
/// branch treats the aborted stream as a successful iteration that produced
/// no tool usage — the follow-up `LlmCompleted` event drives the machine
/// through the standard terminal path (either `Finish` or the next iteration
/// via `AwaitingLlm`, depending on `has_tool_calls`, which is always `false`
/// for repetition aborts).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepetitionAction {
    /// Abort the stream and truncate accumulated text at this byte index.
    ///
    /// The index is guaranteed to land on a UTF-8 character boundary
    /// because the underlying `RepetitionGuard::feed` computes it from
    /// `char_indices` on the caller's accumulated buffer — passing it
    /// directly to `String::truncate` is safe.
    Abort {
        /// Byte offset into the runner's accumulated text buffer at which
        /// to truncate. Points just past the first occurrence of the
        /// repeating probe so the user sees one clean copy of the looped
        /// block.
        truncate_at_byte: usize,
    },
}

/// Classification of an LLM streaming-call failure.
///
/// Each variant maps to a distinct recovery branch in the legacy
/// `run_agent_loop` — preserved here so the sans-IO machine can express
/// the full taxonomy without the runner having to replicate the branching
/// logic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlmFailureKind {
    /// Provider rate limit (HTTP 429 or equivalent). Mirrors the legacy
    /// `is_rate_limit_error` branch: when at least one tool call has
    /// already been made this turn, stop retrying, disable tools, fold,
    /// and inject a "summarize with what you have" nudge; when zero tool
    /// calls have been made the machine falls back to the generic
    /// retryable branch (legacy order: rate-limit check is gated on
    /// `tool_calls_made > 0`, then `is_retryable_provider_error`).
    RateLimited,
    /// Generic retryable provider error (transient 5xx, connection reset,
    /// parse timeouts, etc.). Disables tools and injects a
    /// "reply without tools" nudge before retrying.
    Retryable {
        /// Underlying error message (surfaced to the recovery nudge).
        message: String,
    },
    /// The stream completed without text, tool calls, or usage — the
    /// provider silently dropped the request, usually because the context
    /// window was exceeded on the free tier. Forces an auto-fold before
    /// the retry so the follow-up request fits.
    EmptyStream,
    /// Non-retryable failure (authentication, model not found, …).
    /// Terminates the turn immediately.
    Permanent {
        /// Underlying error message.
        message: String,
    },
}

/// Events fed to [`AgentMachine::step`] by the runner.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Begin a new turn (issued exactly once after construction).
    TurnStarted,
    /// LLM streaming call finished successfully.
    LlmCompleted {
        /// Concatenated assistant text.
        text:           String,
        /// Tool calls extracted from the response (may be empty).
        tool_calls:     Vec<ToolCall>,
        /// Whether the response indicates the LLM wants tools executed.
        has_tool_calls: bool,
    },
    /// LLM streaming call terminated without a usable response.
    ///
    /// The variants of [`LlmFailureKind`] encode the four distinct
    /// recovery branches the legacy `run_agent_loop` distinguishes:
    /// permanent error, retryable transport/server error, provider rate
    /// limit, and silent empty stream (likely context-window overflow).
    LlmFailed {
        /// Failure classification; drives which recovery effects the
        /// machine emits (fold, message injection, retry limits).
        kind: LlmFailureKind,
    },
    /// All tool calls in the current wave have results (success or error).
    ToolsCompleted {
        /// Per-call outcome — order matches the originating
        /// [`Effect::RunTools::calls`].
        results: Vec<ToolResult>,
    },
    /// Security guard rejected the entire wave.
    GuardRejected {
        /// Reason string surfaced to the user / tape.
        reason: String,
    },
    /// User cancelled the turn (Ctrl-C, /stop, kernel shutdown, …).
    Interrupted,
    /// User (or the pause timeout) decided whether to continue after a
    /// tool-call-limit pause.
    LimitResolved {
        /// Id of the pause the decision resolves. Stale ids are ignored
        /// by the machine to prevent a late decision from resuming a
        /// subsequent pause.
        limit_id: u64,
        /// Continue or stop.
        decision: LimitDecision,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agent::effect::{ToolCall as Tc, ToolCallId, ToolResult as Tr},
        tool::ToolName,
    };

    fn tool_call(id: &str, name: &str) -> Tc {
        Tc {
            id:        ToolCallId::new(id),
            name:      ToolName::new(name),
            arguments: "{}".to_owned(),
        }
    }

    fn tool_result(id: &str, name: &str, args: &str, success: bool) -> Tr {
        Tr {
            id: ToolCallId::new(id),
            name: ToolName::new(name),
            arguments: args.to_owned(),
            success,
            duration_ms: 1,
            error: if success { None } else { Some("boom".into()) },
        }
    }

    #[test]
    fn happy_path_text_only() {
        let mut m = AgentMachine::new(8);
        let effects = m.step(Event::TurnStarted);
        assert!(matches!(
            effects.as_slice(),
            [Effect::RebuildTape { .. }, Effect::CallLlm { .. }]
        ));
        assert_eq!(m.phase(), Phase::AwaitingLlm);

        let effects = m.step(Event::LlmCompleted {
            text:           "hi user".into(),
            tool_calls:     vec![],
            has_tool_calls: false,
        });
        assert_eq!(m.phase(), Phase::Done);
        assert!(m.is_terminal());
        assert!(matches!(
            effects.as_slice(),
            [
                Effect::AppendTape {
                    kind: TapeAppendKind::AssistantFinal,
                },
                Effect::EmitCascadeTrace { .. },
                Effect::Finish {
                    reason: FinishReason::Stopped,
                    ..
                },
            ]
        ));
    }

    #[test]
    fn happy_path_with_tool_call_then_stop() {
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);

        let effects = m.step(Event::LlmCompleted {
            text:           "thinking".into(),
            tool_calls:     vec![tool_call("c1", "search")],
            has_tool_calls: true,
        });
        assert_eq!(m.phase(), Phase::ExecutingTools);
        assert_eq!(m.tool_calls_made(), 1);
        assert!(matches!(
            effects.as_slice(),
            [
                Effect::AppendTape {
                    kind: TapeAppendKind::AssistantIntermediate,
                },
                Effect::AppendTape {
                    kind: TapeAppendKind::ToolCalls,
                },
                Effect::RunTools { .. },
            ]
        ));

        let effects = m.step(Event::ToolsCompleted {
            results: vec![tool_result("c1", "search", "{}", true)],
        });
        // Loop continues — runner gets a fresh CallLlm.
        assert_eq!(m.phase(), Phase::AwaitingLlm);
        assert_eq!(m.iteration(), 1);
        assert!(matches!(
            effects.as_slice(),
            [
                Effect::AppendTape {
                    kind: TapeAppendKind::ToolResults,
                },
                Effect::RebuildTape { iteration: 1 },
                Effect::CallLlm { iteration: 1, .. },
            ]
        ));

        // Final LLM call wraps up the turn.
        let _ = m.step(Event::LlmCompleted {
            text:           "done".into(),
            tool_calls:     vec![],
            has_tool_calls: false,
        });
        assert_eq!(m.phase(), Phase::Done);
    }

    #[test]
    fn llm_error_retryable_falls_back_to_no_tools() {
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);

        let effects = m.step(Event::LlmFailed {
            kind: LlmFailureKind::Retryable {
                message: "503".into(),
            },
        });
        // Recovery: machine stays in AwaitingLlm and re-issues CallLlm with tools off.
        assert_eq!(m.phase(), Phase::AwaitingLlm);
        match &effects[..] {
            [
                Effect::InjectUserMessage { text },
                Effect::RebuildTape { .. },
                Effect::CallLlm { tools_enabled, .. },
            ] => {
                assert!(!tools_enabled);
                assert!(text.contains("503"), "nudge should echo error: {text}");
            }
            other => panic!("unexpected effects: {other:?}"),
        }
    }

    #[test]
    fn llm_error_non_retryable_fails_immediately() {
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);

        let effects = m.step(Event::LlmFailed {
            kind: LlmFailureKind::Permanent {
                message: "auth".into(),
            },
        });
        assert_eq!(m.phase(), Phase::Failed);
        assert!(matches!(
            effects.as_slice(),
            [Effect::EmitCascadeTrace { .. }, Effect::Fail { .. }]
        ));
    }

    #[test]
    fn llm_error_exhausts_retries() {
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);
        for _ in 0..MAX_LLM_RECOVERIES {
            let _ = m.step(Event::LlmFailed {
                kind: LlmFailureKind::Retryable {
                    message: "x".into(),
                },
            });
            assert_eq!(m.phase(), Phase::AwaitingLlm);
        }
        let effects = m.step(Event::LlmFailed {
            kind: LlmFailureKind::Retryable {
                message: "x".into(),
            },
        });
        assert_eq!(m.phase(), Phase::Failed);
        assert!(matches!(
            effects.as_slice(),
            [Effect::EmitCascadeTrace { .. }, Effect::Fail { .. }]
        ));
    }

    #[test]
    fn rate_limit_with_tool_calls_folds_and_disables_tools() {
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);
        // Make one tool round-trip so `tool_calls_made > 0`.
        let _ = m.step(Event::LlmCompleted {
            text:           String::new(),
            tool_calls:     vec![tool_call("c1", "search")],
            has_tool_calls: true,
        });
        let _ = m.step(Event::ToolsCompleted {
            results: vec![tool_result("c1", "search", "{}", true)],
        });

        // Now the follow-up LLM call hits a rate limit.
        let effects = m.step(Event::LlmFailed {
            kind: LlmFailureKind::RateLimited,
        });
        assert_eq!(m.phase(), Phase::AwaitingLlm);
        match effects.as_slice() {
            [
                Effect::InjectUserMessage { text },
                Effect::ForceFoldNextIteration,
                Effect::RebuildTape { .. },
                Effect::CallLlm { tools_enabled, .. },
            ] => {
                assert!(!tools_enabled);
                assert!(text.contains("rate limit"), "inject nudge: {text}");
            }
            other => panic!("unexpected effects: {other:?}"),
        }
    }

    #[test]
    fn rate_limit_before_any_tool_falls_through_to_retryable() {
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);
        // First call errors with a rate limit, no tools made yet.
        let effects = m.step(Event::LlmFailed {
            kind: LlmFailureKind::RateLimited,
        });
        assert_eq!(m.phase(), Phase::AwaitingLlm);
        // Retryable branch: inject + CallLlm, no ForceFold.
        assert!(matches!(
            effects.as_slice(),
            [
                Effect::InjectUserMessage { .. },
                Effect::RebuildTape { .. },
                Effect::CallLlm { .. },
            ]
        ));
    }

    #[test]
    fn empty_stream_folds_and_retries() {
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);

        let effects = m.step(Event::LlmFailed {
            kind: LlmFailureKind::EmptyStream,
        });
        assert_eq!(m.phase(), Phase::AwaitingLlm);
        match effects.as_slice() {
            [
                Effect::InjectUserMessage { text },
                Effect::ForceFoldNextIteration,
                Effect::RebuildTape { .. },
                Effect::CallLlm { tools_enabled, .. },
            ] => {
                assert!(!tools_enabled);
                assert!(text.contains("empty response"), "nudge: {text}");
            }
            other => panic!("unexpected effects: {other:?}"),
        }
    }

    #[test]
    fn empty_stream_exhausts_recoveries() {
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);
        for _ in 0..MAX_LLM_RECOVERIES {
            let _ = m.step(Event::LlmFailed {
                kind: LlmFailureKind::EmptyStream,
            });
            assert_eq!(m.phase(), Phase::AwaitingLlm);
        }
        let effects = m.step(Event::LlmFailed {
            kind: LlmFailureKind::EmptyStream,
        });
        assert_eq!(m.phase(), Phase::Failed);
        assert!(matches!(
            effects.as_slice(),
            [Effect::EmitCascadeTrace { .. }, Effect::Fail { .. }]
        ));
    }

    #[test]
    fn tool_failure_still_loops_back_to_llm() {
        // Tool errors are *data*: the runner reports them to the machine via
        // ToolsCompleted, the machine forwards them to the LLM as the next
        // iteration's context.  Failures do NOT abort the turn.
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);
        let _ = m.step(Event::LlmCompleted {
            text:           String::new(),
            tool_calls:     vec![tool_call("c1", "broken")],
            has_tool_calls: true,
        });
        let effects = m.step(Event::ToolsCompleted {
            results: vec![tool_result("c1", "broken", "{}", false)],
        });
        assert_eq!(m.phase(), Phase::AwaitingLlm);
        assert!(matches!(effects.last(), Some(Effect::CallLlm { .. })));
    }

    #[test]
    fn guard_rejection_aborts_turn() {
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);
        let _ = m.step(Event::LlmCompleted {
            text:           String::new(),
            tool_calls:     vec![tool_call("c1", "rm-rf")],
            has_tool_calls: true,
        });
        let effects = m.step(Event::GuardRejected {
            reason: "denied path".into(),
        });
        assert_eq!(m.phase(), Phase::Failed);
        assert!(matches!(
            effects.as_slice(),
            [Effect::EmitCascadeTrace { .. }, Effect::Fail { .. }]
        ));
    }

    #[test]
    fn interruption_from_awaiting_llm_fails() {
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);
        let effects = m.step(Event::Interrupted);
        assert_eq!(m.phase(), Phase::Failed);
        assert!(matches!(
            effects.as_slice(),
            [Effect::EmitCascadeTrace { .. }, Effect::Fail { .. }]
        ));
    }

    #[test]
    fn interruption_from_executing_tools_fails() {
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);
        let _ = m.step(Event::LlmCompleted {
            text:           String::new(),
            tool_calls:     vec![tool_call("c1", "search")],
            has_tool_calls: true,
        });
        let effects = m.step(Event::Interrupted);
        assert_eq!(m.phase(), Phase::Failed);
        assert!(matches!(
            effects.as_slice(),
            [Effect::EmitCascadeTrace { .. }, Effect::Fail { .. }]
        ));
    }

    #[test]
    fn max_iterations_terminates_with_max_reason() {
        let mut m = AgentMachine::new(2);
        let _ = m.step(Event::TurnStarted);
        // Iteration 0: tool call, loop back.
        let _ = m.step(Event::LlmCompleted {
            text:           "step 0".into(),
            tool_calls:     vec![tool_call("c1", "t")],
            has_tool_calls: true,
        });
        let _ = m.step(Event::ToolsCompleted {
            results: vec![tool_result("c1", "t", "{}", true)],
        });
        assert_eq!(m.iteration(), 1);
        // Iteration 1: another tool call.
        let _ = m.step(Event::LlmCompleted {
            text:           "step 1".into(),
            tool_calls:     vec![tool_call("c2", "t")],
            has_tool_calls: true,
        });
        let effects = m.step(Event::ToolsCompleted {
            results: vec![tool_result("c2", "t", "{}", true)],
        });
        // iteration is now 2 == max_iterations → Finish(MaxIterations).
        assert_eq!(m.phase(), Phase::Done);
        assert!(matches!(
            effects.last(),
            Some(Effect::Finish {
                reason: FinishReason::MaxIterations,
                ..
            })
        ));
    }

    #[test]
    fn continuation_signal_loops_back_to_llm() {
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);

        // LLM calls continue-work tool
        let _ = m.step(Event::LlmCompleted {
            text:           "working on it".into(),
            tool_calls:     vec![tool_call("c1", "continue-work")],
            has_tool_calls: true,
        });
        let _ = m.step(Event::ToolsCompleted {
            results: vec![tool_result("c1", "continue-work", "{}", true)],
        });

        // LLM responds with text only — BUT continuation was signaled
        let effects = m.step(Event::LlmCompleted {
            text:           "still working...".into(),
            tool_calls:     vec![],
            has_tool_calls: false,
        });

        // Should NOT terminate — should inject wake and loop back
        assert_eq!(m.phase(), Phase::AwaitingLlm);
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::InjectContinuationWake { .. }))
        );
        assert!(effects.iter().any(|e| matches!(e, Effect::CallLlm { .. })));
        assert_eq!(m.continuation_count(), 1);
    }

    #[test]
    fn continuation_respects_max_limit() {
        let mut m = AgentMachine::with_max_continuations(20, 2);
        let _ = m.step(Event::TurnStarted);

        // Use up 2 continuations
        for i in 0..2 {
            let _ = m.step(Event::LlmCompleted {
                text:           String::new(),
                tool_calls:     vec![tool_call(&format!("c{i}"), "continue-work")],
                has_tool_calls: true,
            });
            let _ = m.step(Event::ToolsCompleted {
                results: vec![tool_result(&format!("c{i}"), "continue-work", "{}", true)],
            });
            // Text-only response — continuation kicks in
            let _ = m.step(Event::LlmCompleted {
                text:           format!("working {i}"),
                tool_calls:     vec![],
                has_tool_calls: false,
            });
            assert_eq!(
                m.phase(),
                Phase::AwaitingLlm,
                "should continue at round {i}"
            );
        }

        // 3rd continue-work call
        let _ = m.step(Event::LlmCompleted {
            text:           String::new(),
            tool_calls:     vec![tool_call("c3", "continue-work")],
            has_tool_calls: true,
        });
        let _ = m.step(Event::ToolsCompleted {
            results: vec![tool_result("c3", "continue-work", "{}", true)],
        });
        // Text-only — but limit reached, should stop
        let effects = m.step(Event::LlmCompleted {
            text:           "done".into(),
            tool_calls:     vec![],
            has_tool_calls: false,
        });
        assert_eq!(m.phase(), Phase::Done);
        assert!(matches!(
            effects.last(),
            Some(Effect::Finish {
                reason: FinishReason::Stopped,
                ..
            })
        ));
    }

    #[test]
    fn continuation_not_triggered_without_signal() {
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);

        // Regular tool call (not continue-work)
        let _ = m.step(Event::LlmCompleted {
            text:           String::new(),
            tool_calls:     vec![tool_call("c1", "search")],
            has_tool_calls: true,
        });
        let _ = m.step(Event::ToolsCompleted {
            results: vec![tool_result("c1", "search", "{}", true)],
        });

        // Text-only — should stop normally
        let effects = m.step(Event::LlmCompleted {
            text:           "here are the results".into(),
            tool_calls:     vec![],
            has_tool_calls: false,
        });
        assert_eq!(m.phase(), Phase::Done);
        assert!(matches!(
            effects.last(),
            Some(Effect::Finish {
                reason: FinishReason::Stopped,
                ..
            })
        ));
        assert_eq!(m.continuation_count(), 0);
    }

    #[test]
    fn invalid_transition_is_surfaced() {
        let mut m = AgentMachine::new(8);
        // Feed ToolsCompleted before any LLM call — pure logic bug.
        let effects = m.step(Event::ToolsCompleted { results: vec![] });
        assert_eq!(m.phase(), Phase::Failed);
        assert!(matches!(
            effects.as_slice(),
            [Effect::EmitCascadeTrace { .. }, Effect::Fail { .. }]
        ));
    }

    // ─── Loop breaker integration ────────────────────────────────────────

    /// Three identical tool calls in a row trip the exact-duplicate detector
    /// (default `exact_dup_threshold = 3`): we expect a `LoopBreakerTriggered`
    /// effect emitted before the next `CallLlm`, and the subsequent `CallLlm`
    /// must carry the newly-disabled tool in its `disabled_tools` field.
    #[test]
    fn loop_breaker_disables_tools_on_exact_duplicate() {
        let mut m = AgentMachine::new(16);
        let _ = m.step(Event::TurnStarted);

        // Drive three waves of the same tool+args to trip exact-duplicate.
        for i in 0..3 {
            let _ = m.step(Event::LlmCompleted {
                text:           "tick".into(),
                tool_calls:     vec![tool_call(&format!("c{i}"), "repeat")],
                has_tool_calls: true,
            });
            let effects = m.step(Event::ToolsCompleted {
                results: vec![tool_result(&format!("c{i}"), "repeat", "{}", true)],
            });
            if i < 2 {
                assert!(
                    !effects
                        .iter()
                        .any(|e| matches!(e, Effect::LoopBreakerTriggered { .. })),
                    "breaker fired too early at wave {i}",
                );
                continue;
            }
            // Wave 3 (i == 2): third identical call → trip.
            let trip = effects
                .iter()
                .find(|e| matches!(e, Effect::LoopBreakerTriggered { .. }))
                .expect("expected LoopBreakerTriggered on third identical wave");
            match trip {
                Effect::LoopBreakerTriggered {
                    pattern,
                    disabled_tools,
                    ..
                } => {
                    assert_eq!(pattern, "exact_duplicate");
                    assert_eq!(disabled_tools, &vec![ToolName::new("repeat")]);
                }
                _ => unreachable!(),
            }
            // The next CallLlm must carry the accumulated disabled_tools.
            let call = effects
                .iter()
                .find_map(|e| match e {
                    Effect::CallLlm { disabled_tools, .. } => Some(disabled_tools),
                    _ => None,
                })
                .expect("expected CallLlm after trip");
            assert_eq!(call, &vec![ToolName::new("repeat")]);
        }
    }

    /// Varying arguments across successive calls keeps the breaker quiet:
    /// different fingerprints, so no exact-duplicate trip and far below
    /// `disable_after = 25`.
    #[test]
    fn loop_breaker_quiet_on_varied_tools() {
        let mut m = AgentMachine::new(16);
        let _ = m.step(Event::TurnStarted);

        for i in 0..3 {
            let _ = m.step(Event::LlmCompleted {
                text:           "tick".into(),
                tool_calls:     vec![tool_call(&format!("c{i}"), "search")],
                has_tool_calls: true,
            });
            let effects = m.step(Event::ToolsCompleted {
                results: vec![tool_result(
                    &format!("c{i}"),
                    "search",
                    &format!(r#"{{"q":"{i}"}}"#),
                    true,
                )],
            });
            assert!(
                !effects
                    .iter()
                    .any(|e| matches!(e, Effect::LoopBreakerTriggered { .. })),
                "breaker fired unexpectedly on varied args at wave {i}",
            );
        }
    }

    /// Once the breaker trips, every subsequent `CallLlm` must continue to
    /// carry the accumulated `disabled_tools` set so the runner can keep
    /// filtering tool definitions across iterations.
    #[test]
    fn disabled_tools_persist_across_iterations() {
        let mut m = AgentMachine::new(16);
        let _ = m.step(Event::TurnStarted);

        // Trip the breaker with three identical calls.
        for i in 0..3 {
            let _ = m.step(Event::LlmCompleted {
                text:           "tick".into(),
                tool_calls:     vec![tool_call(&format!("c{i}"), "repeat")],
                has_tool_calls: true,
            });
            let _ = m.step(Event::ToolsCompleted {
                results: vec![tool_result(&format!("c{i}"), "repeat", "{}", true)],
            });
        }

        // Now run two more iterations with a different tool and verify the
        // disabled set is still threaded through every CallLlm.
        for i in 3..5 {
            let _ = m.step(Event::LlmCompleted {
                text:           "tock".into(),
                tool_calls:     vec![tool_call(&format!("c{i}"), "search")],
                has_tool_calls: true,
            });
            let effects = m.step(Event::ToolsCompleted {
                results: vec![tool_result(
                    &format!("c{i}"),
                    "search",
                    &format!(r#"{{"q":"{i}"}}"#),
                    true,
                )],
            });
            let disabled = effects
                .iter()
                .find_map(|e| match e {
                    Effect::CallLlm { disabled_tools, .. } => Some(disabled_tools.clone()),
                    _ => None,
                })
                .expect("expected CallLlm after iteration");
            assert_eq!(
                disabled,
                vec![ToolName::new("repeat")],
                "disabled set should persist at iteration {i}",
            );
        }
    }

    // ─── Tool-call-limit circuit breaker ────────────────────────────────

    /// With `limit_interval = 1`, the very first tool wave trips the
    /// pause. The machine emits `PauseForLimit` and transitions to
    /// `PausedForLimit`; feeding `Continue` back resumes the turn.
    #[test]
    fn pause_for_limit_fires_at_threshold_and_continue_resumes() {
        let mut m = AgentMachine::with_tool_call_limit(8, 1);
        let _ = m.step(Event::TurnStarted);
        let _ = m.step(Event::LlmCompleted {
            text:           "step".into(),
            tool_calls:     vec![tool_call("c1", "search")],
            has_tool_calls: true,
        });
        let effects = m.step(Event::ToolsCompleted {
            results: vec![tool_result("c1", "search", "{}", true)],
        });
        assert_eq!(m.phase(), Phase::PausedForLimit);
        let pause = effects
            .iter()
            .find_map(|e| match e {
                Effect::PauseForLimit {
                    limit_id,
                    tool_calls_made,
                } => Some((*limit_id, *tool_calls_made)),
                _ => None,
            })
            .expect("expected PauseForLimit effect");
        assert_eq!(pause, (1, 1));
        assert!(
            !effects.iter().any(|e| matches!(e, Effect::CallLlm { .. })),
            "no CallLlm should be emitted while paused",
        );

        let effects = m.step(Event::LimitResolved {
            limit_id: pause.0,
            decision: LimitDecision::Continue,
        });
        assert_eq!(m.phase(), Phase::AwaitingLlm);
        assert!(matches!(
            effects.as_slice(),
            [Effect::RebuildTape { .. }, Effect::CallLlm { .. }]
        ));
    }

    /// `Stop` is a graceful termination: `Finish` with `StoppedByLimit`.
    #[test]
    fn pause_for_limit_stop_terminates_gracefully() {
        let mut m = AgentMachine::with_tool_call_limit(8, 1);
        let _ = m.step(Event::TurnStarted);
        let _ = m.step(Event::LlmCompleted {
            text:           "step".into(),
            tool_calls:     vec![tool_call("c1", "search")],
            has_tool_calls: true,
        });
        let _ = m.step(Event::ToolsCompleted {
            results: vec![tool_result("c1", "search", "{}", true)],
        });
        let effects = m.step(Event::LimitResolved {
            limit_id: 1,
            decision: LimitDecision::Stop,
        });
        assert_eq!(m.phase(), Phase::Done);
        assert!(matches!(
            effects.as_slice(),
            [
                Effect::EmitCascadeTrace { .. },
                Effect::Finish {
                    reason: FinishReason::StoppedByLimit,
                    ..
                },
            ]
        ));
    }

    /// Stale `LimitResolved` (mismatched id) must not advance the machine.
    /// In legacy code this is prevented by the session-scoped oneshot key,
    /// so the machine mirrors that guarantee with the id check.
    #[test]
    fn pause_for_limit_ignores_stale_resolution() {
        let mut m = AgentMachine::with_tool_call_limit(8, 1);
        let _ = m.step(Event::TurnStarted);
        let _ = m.step(Event::LlmCompleted {
            text:           "step".into(),
            tool_calls:     vec![tool_call("c1", "search")],
            has_tool_calls: true,
        });
        let _ = m.step(Event::ToolsCompleted {
            results: vec![tool_result("c1", "search", "{}", true)],
        });
        // Feed a wrong id — falls into the invalid-transition arm.
        let effects = m.step(Event::LimitResolved {
            limit_id: 999,
            decision: LimitDecision::Continue,
        });
        assert_eq!(m.phase(), Phase::Failed);
        assert!(matches!(
            effects.as_slice(),
            [Effect::EmitCascadeTrace { .. }, Effect::Fail { .. }]
        ));
    }

    /// `limit_interval = 0` disables the circuit breaker entirely; the
    /// machine keeps looping and never emits `PauseForLimit`.
    #[test]
    fn zero_interval_disables_limit() {
        let mut m = AgentMachine::with_tool_call_limit(8, 0);
        let _ = m.step(Event::TurnStarted);
        for i in 0..3 {
            let _ = m.step(Event::LlmCompleted {
                text:           "t".into(),
                tool_calls:     vec![tool_call(&format!("c{i}"), "t")],
                has_tool_calls: true,
            });
            let effects = m.step(Event::ToolsCompleted {
                results: vec![tool_result(&format!("c{i}"), "t", "{}", true)],
            });
            assert!(
                !effects
                    .iter()
                    .any(|e| matches!(e, Effect::PauseForLimit { .. })),
                "limit should stay disabled at wave {i}",
            );
            assert_eq!(m.phase(), Phase::AwaitingLlm);
        }
    }

    /// Continue advances `next_limit_at` by `limit_interval`, so the next
    /// pause only fires after another full interval of tool calls.
    #[test]
    fn continue_advances_next_threshold_by_interval() {
        let mut m = AgentMachine::with_tool_call_limit(16, 2);
        let _ = m.step(Event::TurnStarted);

        // Two tool calls → crosses first threshold (2 ≥ 2) → pause.
        let _ = m.step(Event::LlmCompleted {
            text:           "x".into(),
            tool_calls:     vec![tool_call("a", "t"), tool_call("b", "t")],
            has_tool_calls: true,
        });
        let _ = m.step(Event::ToolsCompleted {
            results: vec![
                tool_result("a", "t", "{}", true),
                tool_result("b", "t", r#"{"q":"2"}"#, true),
            ],
        });
        assert_eq!(m.phase(), Phase::PausedForLimit);
        let _ = m.step(Event::LimitResolved {
            limit_id: 1,
            decision: LimitDecision::Continue,
        });

        // One more tool call → at 3, below new threshold (2 + 2 = 4), no pause.
        let _ = m.step(Event::LlmCompleted {
            text:           "y".into(),
            tool_calls:     vec![tool_call("c", "t")],
            has_tool_calls: true,
        });
        let effects = m.step(Event::ToolsCompleted {
            results: vec![tool_result("c", "t", r#"{"q":"3"}"#, true)],
        });
        assert_eq!(m.phase(), Phase::AwaitingLlm);
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::PauseForLimit { .. })),
            "below advanced threshold should not pause",
        );

        // Another call → reaches 4 → pause again with next id.
        let _ = m.step(Event::LlmCompleted {
            text:           "z".into(),
            tool_calls:     vec![tool_call("d", "t")],
            has_tool_calls: true,
        });
        let effects = m.step(Event::ToolsCompleted {
            results: vec![tool_result("d", "t", r#"{"q":"4"}"#, true)],
        });
        assert_eq!(m.phase(), Phase::PausedForLimit);
        let id = effects
            .iter()
            .find_map(|e| match e {
                Effect::PauseForLimit { limit_id, .. } => Some(*limit_id),
                _ => None,
            })
            .expect("expected second PauseForLimit");
        assert_eq!(id, 2, "limit id monotonically increases");
    }

    // ─── Context-pressure observation ───────────────────────────────────

    /// Below the warning threshold the observer stays silent.
    #[test]
    fn context_pressure_silent_below_warning() {
        let mut m = AgentMachine::new(8);
        assert!(m.observe_context_usage(500, 1_000).is_empty());
        assert!(m.observe_context_usage(699, 1_000).is_empty());
    }

    /// Crossing 0.70 (but not 0.85) emits a single Warning effect.
    #[test]
    fn context_pressure_fires_warning_at_threshold() {
        let mut m = AgentMachine::new(8);
        let effects = m.observe_context_usage(750, 1_000);
        match effects.as_slice() {
            [
                Effect::ContextPressureWarning {
                    level: PressureLevel::Warning,
                    estimated_tokens,
                    context_window_tokens,
                },
            ] => {
                assert_eq!(*estimated_tokens, 750);
                assert_eq!(*context_window_tokens, 1_000);
            }
            other => panic!("expected single Warning, got {other:?}"),
        }
    }

    /// Warning is one-shot: repeated observations in the same bucket are
    /// silent even when usage rises further within the Warning band.
    #[test]
    fn context_pressure_warning_is_one_shot() {
        let mut m = AgentMachine::new(8);
        assert_eq!(m.observe_context_usage(750, 1_000).len(), 1);
        assert!(m.observe_context_usage(800, 1_000).is_empty());
        assert!(m.observe_context_usage(849, 1_000).is_empty());
    }

    /// Crossing 0.85 emits Critical even if Warning has already fired; and
    /// the subsequent Warning-band observation is swallowed.
    #[test]
    fn context_pressure_upgrades_to_critical() {
        let mut m = AgentMachine::new(8);
        assert_eq!(m.observe_context_usage(750, 1_000).len(), 1); // Warning
        let effects = m.observe_context_usage(900, 1_000);
        match effects.as_slice() {
            [
                Effect::ContextPressureWarning {
                    level: PressureLevel::Critical,
                    ..
                },
            ] => {}
            other => panic!("expected Critical, got {other:?}"),
        }
        // No more warnings — critical is one-shot too.
        assert!(m.observe_context_usage(950, 1_000).is_empty());
    }

    /// Jumping straight past Warning into Critical emits Critical only and
    /// does not double up with a Warning.
    #[test]
    fn context_pressure_skips_warning_when_jumping_to_critical() {
        let mut m = AgentMachine::new(8);
        let effects = m.observe_context_usage(900, 1_000);
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::ContextPressureWarning {
                level: PressureLevel::Critical,
                ..
            }
        ));
        // A subsequent Warning-band observation must NOT produce a stale
        // Warning effect.
        assert!(m.observe_context_usage(750, 1_000).is_empty());
    }

    /// Zero context window is treated as "unknown" and never emits.
    #[test]
    fn context_pressure_zero_window_is_noop() {
        let mut m = AgentMachine::new(8);
        assert!(m.observe_context_usage(10_000, 0).is_empty());
    }

    /// Mirrors the legacy `run_agent_loop` exemption for read-only tools:
    /// callers pass a `flooding_exempt` set so tools like `search` / `read`
    /// are not disabled after 25 varied-argument invocations. Without this
    /// the machine would regress long read-only investigations once the
    /// runner replaces the legacy loop in production.
    #[test]
    fn loop_breaker_flooding_exempt_is_honoured() {
        use std::collections::HashSet;

        let cfg = LoopBreakerConfig::builder()
            .flooding_exempt(HashSet::from(["search".to_owned()]))
            .build();
        let mut m = AgentMachine::with_loop_breaker_config(200, cfg);
        let _ = m.step(Event::TurnStarted);

        // 30 varied-arg calls — would trip `disable_after = 25` without the
        // exemption.
        for i in 0..30 {
            let _ = m.step(Event::LlmCompleted {
                text:           "tick".into(),
                tool_calls:     vec![tool_call(&format!("c{i}"), "search")],
                has_tool_calls: true,
            });
            let effects = m.step(Event::ToolsCompleted {
                results: vec![tool_result(
                    &format!("c{i}"),
                    "search",
                    &format!(r#"{{"q":"{i}"}}"#),
                    true,
                )],
            });
            assert!(
                !effects
                    .iter()
                    .any(|e| matches!(e, Effect::LoopBreakerTriggered { .. })),
                "exempt tool should not flood at wave {i}",
            );
        }
    }

    /// A successful `discover-tools` call in a mid-turn wave must queue a
    /// [`Effect::RefreshDeferredTools`] right before the next
    /// [`Effect::CallLlm`] so the upcoming LLM call sees the freshly
    /// activated catalog.
    #[test]
    fn discover_tools_emits_refresh_before_next_llm_call() {
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);
        let _ = m.step(Event::LlmCompleted {
            text:           String::new(),
            tool_calls:     vec![tool_call("c1", DISCOVER_TOOLS_TOOL_NAME)],
            has_tool_calls: true,
        });

        let effects = m.step(Event::ToolsCompleted {
            results: vec![tool_result(
                "c1",
                DISCOVER_TOOLS_TOOL_NAME,
                r#"{"query":"fs"}"#,
                true,
            )],
        });

        assert_eq!(m.phase(), Phase::AwaitingLlm);
        let refresh_idx = effects
            .iter()
            .position(|e| matches!(e, Effect::RefreshDeferredTools { .. }))
            .expect("expected RefreshDeferredTools");
        let call_idx = effects
            .iter()
            .position(|e| matches!(e, Effect::CallLlm { .. }))
            .expect("expected follow-up CallLlm");
        assert!(
            refresh_idx < call_idx,
            "refresh must precede next CallLlm: {effects:?}"
        );
        match &effects[refresh_idx] {
            Effect::RefreshDeferredTools { trigger_call_ids } => {
                assert_eq!(
                    trigger_call_ids,
                    &vec![crate::agent::effect::ToolCallId::new("c1")]
                );
            }
            other => panic!("unexpected effect variant: {other:?}"),
        }
    }

    /// Collect every successful `discover-tools` call in a mixed wave while
    /// ignoring failed and unrelated calls.
    #[test]
    fn discover_tools_mixed_wave_only_forwards_successful_ids() {
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);
        let _ = m.step(Event::LlmCompleted {
            text:           String::new(),
            tool_calls:     vec![
                tool_call("a", DISCOVER_TOOLS_TOOL_NAME),
                tool_call("b", "search"),
                tool_call("c", DISCOVER_TOOLS_TOOL_NAME),
            ],
            has_tool_calls: true,
        });

        let effects = m.step(Event::ToolsCompleted {
            results: vec![
                tool_result("a", DISCOVER_TOOLS_TOOL_NAME, "{}", true),
                tool_result("b", "search", "{}", true),
                // A failed discover-tools must not trigger activation.
                tool_result("c", DISCOVER_TOOLS_TOOL_NAME, "{}", false),
            ],
        });

        let refresh = effects
            .iter()
            .find_map(|e| match e {
                Effect::RefreshDeferredTools { trigger_call_ids } => Some(trigger_call_ids),
                _ => None,
            })
            .expect("expected RefreshDeferredTools");
        assert_eq!(
            refresh,
            &vec![crate::agent::effect::ToolCallId::new("a")],
            "only successful discover-tools ids should propagate"
        );
    }

    /// Waves that don't include a successful `discover-tools` call must NOT
    /// emit a refresh effect — the runner would otherwise redo work for
    /// every iteration.
    #[test]
    fn no_refresh_when_discover_tools_absent() {
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);
        let _ = m.step(Event::LlmCompleted {
            text:           String::new(),
            tool_calls:     vec![tool_call("c1", "search")],
            has_tool_calls: true,
        });
        let effects = m.step(Event::ToolsCompleted {
            results: vec![tool_result("c1", "search", "{}", true)],
        });
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::RefreshDeferredTools { .. })),
            "no refresh expected: {effects:?}"
        );
    }

    /// A terminal wave that hits `max_iterations` must NOT emit a refresh —
    /// there's no upcoming LLM call to consume the activation set.
    #[test]
    fn terminal_max_iterations_wave_skips_refresh() {
        let mut m = AgentMachine::new(1);
        let _ = m.step(Event::TurnStarted);
        let _ = m.step(Event::LlmCompleted {
            text:           "final".into(),
            tool_calls:     vec![tool_call("c1", DISCOVER_TOOLS_TOOL_NAME)],
            has_tool_calls: true,
        });
        let effects = m.step(Event::ToolsCompleted {
            results: vec![tool_result("c1", DISCOVER_TOOLS_TOOL_NAME, "{}", true)],
        });
        assert_eq!(m.phase(), Phase::Done);
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::RefreshDeferredTools { .. })),
            "refresh should be suppressed on terminal wave: {effects:?}"
        );
    }

    // ─── Per-iteration tape rebuild + sanitisation ──────────────────────

    /// Helper: count `Effect::RebuildTape` occurrences in a slice.
    fn count_rebuilds(effects: &[Effect]) -> usize {
        effects
            .iter()
            .filter(|e| matches!(e, Effect::RebuildTape { .. }))
            .count()
    }

    /// Every `CallLlm` the machine emits must be immediately preceded by a
    /// matching `RebuildTape`. The legacy `run_agent_loop` rebuilds the
    /// message list from the tape at the top of every iteration; this
    /// invariant is what makes the tape (not an in-memory buffer) the
    /// single source of truth for conversation history.
    #[test]
    fn rebuild_tape_precedes_initial_call_llm() {
        let mut m = AgentMachine::new(8);
        let effects = m.step(Event::TurnStarted);
        let call_idx = effects
            .iter()
            .position(|e| matches!(e, Effect::CallLlm { .. }))
            .expect("expected CallLlm on turn boot");
        let rebuild_idx = effects
            .iter()
            .position(|e| matches!(e, Effect::RebuildTape { .. }))
            .expect("expected RebuildTape on turn boot");
        assert!(
            rebuild_idx + 1 == call_idx,
            "rebuild must sit directly before CallLlm: {effects:?}"
        );
        match &effects[rebuild_idx] {
            Effect::RebuildTape { iteration } => assert_eq!(*iteration, 0),
            _ => unreachable!(),
        }
    }

    /// After a tool wave, the next iteration's `CallLlm` carries the
    /// bumped iteration number and the preceding `RebuildTape` shares it.
    #[test]
    fn rebuild_tape_precedes_post_tool_call_llm_with_matching_iteration() {
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);
        let _ = m.step(Event::LlmCompleted {
            text:           "thinking".into(),
            tool_calls:     vec![tool_call("c1", "search")],
            has_tool_calls: true,
        });
        let effects = m.step(Event::ToolsCompleted {
            results: vec![tool_result("c1", "search", "{}", true)],
        });
        let rebuild = effects
            .iter()
            .find_map(|e| match e {
                Effect::RebuildTape { iteration } => Some(*iteration),
                _ => None,
            })
            .expect("expected RebuildTape on next iteration");
        assert_eq!(rebuild, 1);
        // And it must still sit directly before the next CallLlm.
        let rebuild_idx = effects
            .iter()
            .position(|e| matches!(e, Effect::RebuildTape { .. }))
            .unwrap();
        let call_idx = effects
            .iter()
            .position(|e| matches!(e, Effect::CallLlm { .. }))
            .unwrap();
        assert!(rebuild_idx + 1 == call_idx, "effects: {effects:?}");
    }

    /// The continuation-wake path issues an extra `CallLlm`; it MUST also
    /// be preceded by a rebuild so the wake message (written to the tape)
    /// is visible to the LLM.
    #[test]
    fn rebuild_tape_precedes_continuation_wake_call() {
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);
        let _ = m.step(Event::LlmCompleted {
            text:           String::new(),
            tool_calls:     vec![tool_call("c1", "continue-work")],
            has_tool_calls: true,
        });
        let _ = m.step(Event::ToolsCompleted {
            results: vec![tool_result("c1", "continue-work", "{}", true)],
        });
        let effects = m.step(Event::LlmCompleted {
            text:           "still working".into(),
            tool_calls:     vec![],
            has_tool_calls: false,
        });

        // Ordering within this batch: AppendTape(Intermediate),
        // InjectContinuationWake, RebuildTape, CallLlm.
        let wake_idx = effects
            .iter()
            .position(|e| matches!(e, Effect::InjectContinuationWake { .. }))
            .expect("expected wake");
        let rebuild_idx = effects
            .iter()
            .position(|e| matches!(e, Effect::RebuildTape { .. }))
            .expect("expected rebuild");
        let call_idx = effects
            .iter()
            .position(|e| matches!(e, Effect::CallLlm { .. }))
            .expect("expected call");
        assert!(
            wake_idx < rebuild_idx && rebuild_idx + 1 == call_idx,
            "effects: {effects:?}"
        );
    }

    /// The LimitResolved::Continue resume path re-enters `CallLlm`; the
    /// rebuild must still prefix it so any nudges pushed during the pause
    /// (pressure warnings, discover-tools refresh) land in the rebuilt
    /// message list.
    #[test]
    fn rebuild_tape_precedes_resume_after_limit_pause() {
        let mut m = AgentMachine::with_tool_call_limit(8, 1);
        let _ = m.step(Event::TurnStarted);
        let _ = m.step(Event::LlmCompleted {
            text:           String::new(),
            tool_calls:     vec![tool_call("c1", "search")],
            has_tool_calls: true,
        });
        let _ = m.step(Event::ToolsCompleted {
            results: vec![tool_result("c1", "search", "{}", true)],
        });
        let effects = m.step(Event::LimitResolved {
            limit_id: 1,
            decision: LimitDecision::Continue,
        });
        assert!(matches!(
            effects.as_slice(),
            [Effect::RebuildTape { .. }, Effect::CallLlm { .. }]
        ));
    }

    /// Every LLM recovery path (retryable / rate-limit-with-tools / empty
    /// stream) retries via `CallLlm` and MUST rebuild first so the
    /// injected nudge is part of the rebuilt messages.
    #[test]
    fn rebuild_tape_precedes_each_recovery_call() {
        use LlmFailureKind::*;
        // Retryable: [Inject, Rebuild, Call]
        {
            let mut m = AgentMachine::new(8);
            let _ = m.step(Event::TurnStarted);
            let effects = m.step(Event::LlmFailed {
                kind: Retryable {
                    message: "503".into(),
                },
            });
            assert_eq!(count_rebuilds(&effects), 1, "{effects:?}");
        }
        // Empty stream: [Inject, ForceFold, Rebuild, Call]
        {
            let mut m = AgentMachine::new(8);
            let _ = m.step(Event::TurnStarted);
            let effects = m.step(Event::LlmFailed { kind: EmptyStream });
            assert_eq!(count_rebuilds(&effects), 1, "{effects:?}");
        }
        // Rate-limit with tools: [Inject, ForceFold, Rebuild, Call]
        {
            let mut m = AgentMachine::new(8);
            let _ = m.step(Event::TurnStarted);
            let _ = m.step(Event::LlmCompleted {
                text:           String::new(),
                tool_calls:     vec![tool_call("c1", "search")],
                has_tool_calls: true,
            });
            let _ = m.step(Event::ToolsCompleted {
                results: vec![tool_result("c1", "search", "{}", true)],
            });
            let effects = m.step(Event::LlmFailed { kind: RateLimited });
            assert_eq!(count_rebuilds(&effects), 1, "{effects:?}");
        }
    }

    /// Terminal waves (Finish on max iterations, PauseForLimit, Stopped)
    /// must NOT emit a rebuild: there is no upcoming CallLlm to consume
    /// it. Emitting one would cause the runner to do unnecessary work and
    /// risk overwriting a buffer that will never be read.
    #[test]
    fn rebuild_tape_absent_on_terminal_waves() {
        // Max iterations → Finish
        {
            let mut m = AgentMachine::new(1);
            let _ = m.step(Event::TurnStarted);
            let _ = m.step(Event::LlmCompleted {
                text:           "x".into(),
                tool_calls:     vec![tool_call("c1", "t")],
                has_tool_calls: true,
            });
            let effects = m.step(Event::ToolsCompleted {
                results: vec![tool_result("c1", "t", "{}", true)],
            });
            assert_eq!(m.phase(), Phase::Done);
            assert_eq!(count_rebuilds(&effects), 0, "{effects:?}");
        }
        // Text-only Stopped → only AppendTape + Finish, no rebuild
        {
            let mut m = AgentMachine::new(8);
            let _ = m.step(Event::TurnStarted);
            let effects = m.step(Event::LlmCompleted {
                text:           "done".into(),
                tool_calls:     vec![],
                has_tool_calls: false,
            });
            assert_eq!(count_rebuilds(&effects), 0, "{effects:?}");
        }
        // Tool-call-limit pause → no rebuild yet (the resume will emit one)
        {
            let mut m = AgentMachine::with_tool_call_limit(8, 1);
            let _ = m.step(Event::TurnStarted);
            let _ = m.step(Event::LlmCompleted {
                text:           String::new(),
                tool_calls:     vec![tool_call("c1", "t")],
                has_tool_calls: true,
            });
            let effects = m.step(Event::ToolsCompleted {
                results: vec![tool_result("c1", "t", "{}", true)],
            });
            assert_eq!(m.phase(), Phase::PausedForLimit);
            assert_eq!(count_rebuilds(&effects), 0, "{effects:?}");
        }
    }

    /// When a discover-tools wave also trips the tool-call limit, the refresh
    /// must precede the pause so that on resume the stored activation set is
    /// already in place for the next `CallLlm`.
    #[test]
    fn refresh_precedes_pause_for_limit() {
        let mut m = AgentMachine::with_tool_call_limit(8, 1);
        let _ = m.step(Event::TurnStarted);
        let _ = m.step(Event::LlmCompleted {
            text:           String::new(),
            tool_calls:     vec![tool_call("c1", DISCOVER_TOOLS_TOOL_NAME)],
            has_tool_calls: true,
        });
        let effects = m.step(Event::ToolsCompleted {
            results: vec![tool_result("c1", DISCOVER_TOOLS_TOOL_NAME, "{}", true)],
        });
        let refresh_idx = effects
            .iter()
            .position(|e| matches!(e, Effect::RefreshDeferredTools { .. }))
            .expect("expected RefreshDeferredTools");
        let pause_idx = effects
            .iter()
            .position(|e| matches!(e, Effect::PauseForLimit { .. }))
            .expect("expected PauseForLimit");
        assert!(
            refresh_idx < pause_idx,
            "refresh must precede pause: {effects:?}"
        );
    }

    // ─── Repetition guard (observe_stream_delta) ────────────────────────

    /// Build a string of `n` characters that is globally unique — no
    /// 200-char slice repeats. Mirrors the helper in `repetition::tests`.
    fn unique_chars(n: usize) -> String {
        let mut s = String::with_capacity(n * 4);
        let mut i = 0u32;
        while s.chars().count() < n {
            let mut num = i;
            let mut buf = Vec::new();
            loop {
                buf.push(b'a' + (num % 26) as u8);
                num /= 26;
                if num == 0 {
                    break;
                }
            }
            buf.reverse();
            for &b in &buf {
                if s.chars().count() >= n {
                    break;
                }
                s.push(char::from(b));
            }
            if s.chars().count() < n {
                s.push('_');
            }
            i += 1;
        }
        s.chars().take(n).collect()
    }

    #[test]
    fn observe_stream_delta_returns_none_for_unique_text() {
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);
        let text = unique_chars(800);
        assert!(m.observe_stream_delta(&text, &text).is_none());
    }

    #[test]
    fn observe_stream_delta_returns_none_below_min_length() {
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);
        // Well below MIN_CHECK_LEN (600) — even pure repetition is skipped.
        let block = "x".repeat(400);
        assert!(m.observe_stream_delta(&block, &block).is_none());
    }

    #[test]
    fn observe_stream_delta_detects_exact_block_repetition() {
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);
        let block = unique_chars(300);
        let repeated = format!("{block}{block}");
        match m.observe_stream_delta(&repeated, &repeated) {
            Some(RepetitionAction::Abort { truncate_at_byte }) => {
                assert!(
                    truncate_at_byte <= block.len() + 200,
                    "truncate_at_byte {truncate_at_byte} should be at most one block + probe"
                );
            }
            None => panic!("expected Abort, got None"),
        }
    }

    /// Incremental feeding (one delta at a time) must produce the same
    /// abort signal as a single bulk feed — this is the realistic runtime
    /// shape: providers emit 5–50 byte deltas, not 600-byte chunks.
    #[test]
    fn observe_stream_delta_fires_on_incremental_feeding() {
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);
        let block = unique_chars(300);
        let combined = format!("{block}{block}{block}");
        let chars: Vec<char> = combined.chars().collect();
        let mut acc = String::new();
        let mut detected = false;
        for chunk in chars.chunks(100) {
            let delta: String = chunk.iter().collect();
            acc.push_str(&delta);
            if m.observe_stream_delta(&delta, &acc).is_some() {
                detected = true;
                break;
            }
        }
        assert!(detected, "incremental feeding must eventually fire");
    }

    /// The guard state is per-LLM-round: every transition that re-enters
    /// `AwaitingLlm` via `rebuild_then_call` wipes the accumulator so the
    /// next iteration starts from zero. Without this reset, the byte-count
    /// `debug_assert` in `RepetitionGuard::feed` would trip on the second
    /// iteration's first delta.
    #[test]
    fn observe_stream_delta_resets_per_llm_round() {
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);

        // Iteration 0: feed some text that doesn't trigger (below MIN_CHECK_LEN
        // means no detection but bytes/chars still accumulate internally).
        let half = "abc".repeat(50); // 150 chars
        let _ = m.observe_stream_delta(&half, &half);

        // Finish iteration 0 with a tool call and advance to iteration 1.
        let _ = m.step(Event::LlmCompleted {
            text:           half.clone(),
            tool_calls:     vec![tool_call("c1", "search")],
            has_tool_calls: true,
        });
        let _ = m.step(Event::ToolsCompleted {
            results: vec![tool_result("c1", "search", "{}", true)],
        });
        // Now in AwaitingLlm for iteration 1. A fresh feed of equal length
        // must NOT trip the internal byte-drift debug_assert — the guard was
        // reset by `rebuild_then_call`.
        let fresh = "xyz".repeat(50);
        assert!(m.observe_stream_delta(&fresh, &fresh).is_none());
    }

    /// Two independent iterations each surface their own repetition — the
    /// reset per round does not swallow a later iteration's loop.
    #[test]
    fn observe_stream_delta_fires_in_later_iteration_after_reset() {
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);

        // Iteration 0: complete a tool round so the machine advances.
        let _ = m.step(Event::LlmCompleted {
            text:           String::new(),
            tool_calls:     vec![tool_call("c1", "search")],
            has_tool_calls: true,
        });
        let _ = m.step(Event::ToolsCompleted {
            results: vec![tool_result("c1", "search", "{}", true)],
        });
        assert_eq!(m.phase(), Phase::AwaitingLlm);

        // Iteration 1: feed a repeating stream — abort must fire here.
        let block = unique_chars(300);
        let repeated = format!("{block}{block}");
        assert!(
            matches!(
                m.observe_stream_delta(&repeated, &repeated),
                Some(RepetitionAction::Abort { .. })
            ),
            "expected Abort on iteration 1"
        );
    }

    // ── Auto-fold ──────────────────────────────────────────────────────

    fn auto_fold_cfg() -> AutoFoldConfig {
        AutoFoldConfig {
            fold_threshold:            0.60,
            min_entries_between_folds: 15,
        }
    }

    #[test]
    fn observer_requests_fold_on_pressure_with_cooldown_ok() {
        let mut m = AgentMachine::with_auto_fold(8, auto_fold_cfg());
        // 70% usage, 20 entries since last fold — both thresholds met.
        assert!(m.observe_fold_pressure(7_000, 10_000, 20));
        assert!(m.force_fold_pending());

        // Next CallLlm boundary emits ForceFoldNextIteration first.
        let effects = m.step(Event::TurnStarted);
        assert!(matches!(
            effects.as_slice(),
            [
                Effect::ForceFoldNextIteration,
                Effect::RebuildTape { .. },
                Effect::CallLlm { .. },
            ]
        ));
        // Flag cleared after emission.
        assert!(!m.force_fold_pending());
    }

    #[test]
    fn observer_skips_below_pressure_threshold() {
        let mut m = AgentMachine::with_auto_fold(8, auto_fold_cfg());
        // 55% usage: strictly below 0.60 threshold.
        assert!(!m.observe_fold_pressure(5_500, 10_000, 100));
        assert!(!m.force_fold_pending());
    }

    #[test]
    fn observer_skips_when_cooldown_not_elapsed() {
        let mut m = AgentMachine::with_auto_fold(8, auto_fold_cfg());
        // 80% usage but only 5 entries since last fold — still inside cooldown.
        assert!(!m.observe_fold_pressure(8_000, 10_000, 5));
        assert!(!m.force_fold_pending());
    }

    #[test]
    fn observer_is_noop_without_config() {
        let mut m = AgentMachine::new(8);
        assert!(!m.observe_fold_pressure(9_500, 10_000, 1000));
        assert!(!m.force_fold_pending());
    }

    #[test]
    fn request_force_fold_bypasses_cooldown() {
        let mut m = AgentMachine::with_auto_fold(8, auto_fold_cfg());
        // No pressure, no cooldown — a ToolHint::SuggestFold caller should
        // still win through `request_force_fold` because trusted tools
        // elect folds on their own.
        assert!(m.request_force_fold());
        assert!(m.force_fold_pending());
    }

    #[test]
    fn request_force_fold_is_noop_without_config() {
        let mut m = AgentMachine::new(8);
        assert!(!m.request_force_fold());
        assert!(!m.force_fold_pending());
    }

    #[test]
    fn mark_fold_failed_latches_off_further_requests() {
        let mut m = AgentMachine::with_auto_fold(8, auto_fold_cfg());
        assert!(m.observe_fold_pressure(9_000, 10_000, 100));
        m.mark_fold_failed();
        assert!(m.fold_disabled());
        // Pending flag cleared so the next CallLlm does NOT emit a stale
        // ForceFoldNextIteration.
        assert!(!m.force_fold_pending());

        // Further observer + request calls are no-ops.
        assert!(!m.observe_fold_pressure(9_500, 10_000, 100));
        assert!(!m.request_force_fold());
        assert!(!m.force_fold_pending());

        let effects = m.step(Event::TurnStarted);
        assert!(matches!(
            effects.as_slice(),
            [Effect::RebuildTape { .. }, Effect::CallLlm { .. }]
        ));
    }

    #[test]
    fn pending_fold_survives_tool_wave_and_fires_on_next_llm() {
        let mut m = AgentMachine::with_auto_fold(8, auto_fold_cfg());
        let _ = m.step(Event::TurnStarted);
        let _ = m.step(Event::LlmCompleted {
            text:           String::new(),
            tool_calls:     vec![tool_call("c1", "search")],
            has_tool_calls: true,
        });

        // Mid-turn: a summariser tool elects a fold.
        assert!(m.request_force_fold());

        // Tool wave finishes — the follow-up CallLlm should carry the fold.
        let effects = m.step(Event::ToolsCompleted {
            results: vec![tool_result("c1", "search", "{}", true)],
        });
        let fold_before_rebuild = effects.windows(2).any(|w| {
            matches!(&w[0], Effect::ForceFoldNextIteration)
                && matches!(&w[1], Effect::RebuildTape { .. })
        });
        assert!(
            fold_before_rebuild,
            "expected [ForceFoldNextIteration, RebuildTape] adjacent in {effects:?}"
        );
        assert!(matches!(effects.last(), Some(Effect::CallLlm { .. })));
        assert!(!m.force_fold_pending(), "flag must clear after emission");
    }

    #[test]
    fn recovery_path_still_requests_fold_when_auto_fold_disabled() {
        // No auto-fold config; EmptyStream recovery must still request the
        // fold — matches the legacy loop where stream recovery fires the
        // flag regardless of `context_folding.enabled`.
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);
        let effects = m.step(Event::LlmFailed {
            kind: LlmFailureKind::EmptyStream,
        });
        match effects.as_slice() {
            [
                Effect::InjectUserMessage { .. },
                Effect::ForceFoldNextIteration,
                Effect::RebuildTape { .. },
                Effect::CallLlm { tools_enabled, .. },
            ] => assert!(!tools_enabled),
            other => panic!("unexpected effects: {other:?}"),
        }
        assert!(
            !m.force_fold_pending(),
            "flag must be cleared after emission"
        );
    }

    #[test]
    fn recovery_path_ignores_fold_disabled_latch() {
        let mut m = AgentMachine::with_auto_fold(8, auto_fold_cfg());
        m.mark_fold_failed();
        let _ = m.step(Event::TurnStarted);
        let effects = m.step(Event::LlmFailed {
            kind: LlmFailureKind::EmptyStream,
        });
        // Legacy parity: recovery branches still emit ForceFoldNextIteration
        // even after a prior fold failure — the subsystem decides whether
        // to actually fold.
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::ForceFoldNextIteration)),
            "recovery must still emit ForceFold despite fold_disabled, got {effects:?}"
        );
    }

    #[test]
    fn observer_emits_fold_only_once_per_boundary() {
        let mut m = AgentMachine::with_auto_fold(8, auto_fold_cfg());
        assert!(m.observe_fold_pressure(9_000, 10_000, 100));
        // Second observer call while flag is still pending is idempotent.
        assert!(m.observe_fold_pressure(9_500, 10_000, 100));
        let effects = m.step(Event::TurnStarted);
        let fold_count = effects
            .iter()
            .filter(|e| matches!(e, Effect::ForceFoldNextIteration))
            .count();
        assert_eq!(fold_count, 1, "exactly one ForceFold per boundary");
    }

    // ---- Cascade trace + mood tests -----------------------------------

    /// Full multi-round turn: user input + assistant thought + tool call +
    /// tool result + final assistant text should all surface in the single
    /// `EmitCascadeTrace` emitted just before `Finish`.
    #[test]
    fn cascade_trace_accumulates_full_turn() {
        use crate::cascade::CascadeEntryKind;

        let mut m = AgentMachine::new(8);
        m.set_cascade_message_id("msg-xyz".to_owned());
        m.observe_user_input("太好了 please search awesome!");

        let _ = m.step(Event::TurnStarted);

        // Round 1: thought + tool call.
        let _ = m.step(Event::LlmCompleted {
            text:           "thinking hard".into(),
            tool_calls:     vec![tool_call("c1", "search")],
            has_tool_calls: true,
        });

        // Tool results observed.
        let _ = m.step(Event::ToolsCompleted {
            results: vec![tool_result("c1", "search", "{\"q\":\"x\"}", true)],
        });

        // Final round: terminal assistant text.
        let effects = m.step(Event::LlmCompleted {
            text:           "太好了 awesome all done!".into(),
            tool_calls:     vec![],
            has_tool_calls: false,
        });

        // The terminal vector must contain exactly one EmitCascadeTrace,
        // positioned immediately before Finish.
        let mut emit_idx = None;
        let mut finish_idx = None;
        for (i, eff) in effects.iter().enumerate() {
            match eff {
                Effect::EmitCascadeTrace { .. } => emit_idx = Some(i),
                Effect::Finish { .. } => finish_idx = Some(i),
                _ => {}
            }
        }
        let emit_idx = emit_idx.expect("EmitCascadeTrace missing");
        let finish_idx = finish_idx.expect("Finish missing");
        assert_eq!(
            emit_idx + 1,
            finish_idx,
            "EmitCascadeTrace must precede Finish"
        );

        let Effect::EmitCascadeTrace { trace, mood } = &effects[emit_idx] else {
            unreachable!()
        };
        assert_eq!(trace.message_id, "msg-xyz");
        assert_eq!(trace.ticks.len(), 2, "expect two ticks: {trace:?}");

        // Round 0 must contain user input + thought + action + observation.
        let round0 = &trace.ticks[0];
        assert!(round0.entries.iter().any(|e| {
            e.kind == CascadeEntryKind::UserInput && e.content.contains("search awesome")
        }));
        assert!(
            round0
                .entries
                .iter()
                .any(|e| e.kind == CascadeEntryKind::Thought && e.content == "thinking hard")
        );
        assert!(
            round0
                .entries
                .iter()
                .any(|e| e.kind == CascadeEntryKind::Action && e.content.contains("search"))
        );
        assert!(
            round0
                .entries
                .iter()
                .any(|e| e.kind == CascadeEntryKind::Observation)
        );

        // Round 1 carries the final assistant text as a Thought entry.
        let round1 = &trace.ticks[1];
        assert!(
            round1
                .entries
                .iter()
                .any(|e| e.kind == CascadeEntryKind::Thought && e.content.contains("awesome"))
        );

        // Mood inference over assistant text tail should pick "cheerful".
        let mood = mood.as_ref().expect("mood must be inferred");
        assert_eq!(mood.label, "cheerful");
        assert!(mood.confidence > 0.3);
    }

    /// The machine must latch `cascade_emitted` so a second terminal step
    /// (e.g. late event arriving at `Phase::Done`) does not re-emit.
    #[test]
    fn cascade_trace_emitted_only_once() {
        let mut m = AgentMachine::new(8);
        m.observe_user_input("hi");
        let _ = m.step(Event::TurnStarted);

        let effects = m.step(Event::LlmCompleted {
            text:           "bye".into(),
            tool_calls:     vec![],
            has_tool_calls: false,
        });
        let first_count = effects
            .iter()
            .filter(|e| matches!(e, Effect::EmitCascadeTrace { .. }))
            .count();
        assert_eq!(first_count, 1);

        // A follow-up Interrupted event on an already-Done machine must
        // not produce another EmitCascadeTrace.
        let effects = m.step(Event::Interrupted);
        let second_count = effects
            .iter()
            .filter(|e| matches!(e, Effect::EmitCascadeTrace { .. }))
            .count();
        assert_eq!(second_count, 0, "EmitCascadeTrace must latch once per turn");
    }

    /// A turn that fails without any assistant text still emits a cascade
    /// trace (possibly empty body) but the mood is `None` because the
    /// assistant-text tail is empty.
    #[test]
    fn cascade_trace_on_failure_carries_no_mood() {
        let mut m = AgentMachine::new(8);
        m.observe_user_input("hi");
        let _ = m.step(Event::TurnStarted);

        let effects = m.step(Event::LlmFailed {
            kind: LlmFailureKind::Permanent {
                message: "auth".into(),
            },
        });
        let emit = effects
            .iter()
            .find_map(|e| match e {
                Effect::EmitCascadeTrace { trace, mood } => Some((trace, mood)),
                _ => None,
            })
            .expect("EmitCascadeTrace missing on failure");
        assert!(
            emit.1.is_none(),
            "no assistant text → no mood: {:?}",
            emit.1
        );
        // User-input entry must still be present so downstream tracing is
        // not lying about what triggered the turn.
        assert!(!emit.0.ticks.is_empty());
    }
}
