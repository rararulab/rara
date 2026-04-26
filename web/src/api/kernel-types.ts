/*
 * Copyright 2025 Rararulab
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

/**
 * Shared types between the kernel REST/WS API and the UI.
 *
 * The kernel emits two data shapes for an agent's execution:
 *   1. Historical turns via `GET /api/v1/kernel/sessions/{key}/turns`
 *      — structured `TurnTrace[]` with completed iterations and tool calls.
 *   2. Live incremental events via WebSocket
 *      `/api/v1/kernel/sessions/{key}/stream` — `StreamEvent` deltas.
 *
 * The UI should not care which source an event came from. This module
 * defines the unified {@link TimelineItem} shape and the helpers that
 * project the two backend shapes into it.
 */

// ---------------------------------------------------------------------------
// Backend-mirrored types
// ---------------------------------------------------------------------------

/** A single tool invocation recorded on a completed iteration. */
export interface ToolCallTrace {
  name: string;
  id: string;
  duration_ms: number;
  success: boolean;
  arguments: Record<string, unknown>;
  result_preview: string;
  error: string | null;
}

/** One iteration of the agent loop (an LLM call + its resulting tool calls). */
export interface IterationTrace {
  index: number;
  first_token_ms: number | null;
  stream_ms: number;
  text_preview: string;
  reasoning_text: string | null;
  tool_calls: ToolCallTrace[];
}

/** Trace data for a single completed agent turn. */
export interface TurnTrace {
  duration_ms: number;
  model: string;
  input_text: string | null;
  iterations: IterationTrace[];
  final_text_len: number;
  total_tool_calls: number;
  success: boolean;
  error: string | null;
}

/**
 * Structured per-step status carried by `plan_progress` events.
 *
 * Mirrors `rara_kernel::io::PlanStepStatus` (snake_case serde tag).
 * `failed` / `needs_replan` carry a human-readable reason that the UI
 * surfaces on the corresponding step row.
 */
export type PlanStepStatusEvent =
  | 'running'
  | 'done'
  | { failed: { reason: string } }
  | { needs_replan: { reason: string } };

/**
 * Narrow subset of kernel `StreamEvent` the UI consumes today.
 *
 * The kernel emits more variants (UsageUpdate, BackgroundTaskStarted,
 * etc.); we parse defensively — unknown variants are ignored by the
 * hook.
 */
export type StreamEvent =
  | { type: 'text_delta'; text: string }
  | { type: 'reasoning_delta'; text: string }
  | { type: 'text_clear' }
  | {
      type: 'tool_call_start';
      name: string;
      id: string;
      arguments: Record<string, unknown>;
    }
  | {
      type: 'tool_call_end';
      id: string;
      result_preview: string;
      success: boolean;
      error: string | null;
    }
  | {
      type: 'turn_metrics';
      duration_ms: number;
      iterations: number;
      tool_calls: number;
      model: string;
      rara_message_id: string;
    }
  | {
      type: 'plan_created';
      goal: string;
      total_steps: number;
      compact_summary: string;
      estimated_duration_secs: number | null;
    }
  | {
      type: 'plan_progress';
      current_step: number;
      total_steps: number;
      step_status: PlanStepStatusEvent;
      status_text: string;
    }
  | { type: 'plan_replan'; reason: string }
  | { type: 'plan_completed'; summary: string }
  /**
   * Settled per-turn token usage. Emitted once near turn end by the web
   * channel adapter, which maps `StreamEvent::TurnUsage` → `WebEvent::Usage`
   * (see `crates/channels/src/web.rs`). `input` is the largest iteration's
   * prompt size; `output` is cumulative completion tokens across the turn.
   */
  | {
      type: 'usage';
      input: number;
      output: number;
      cache_read: number;
      cache_write: number;
      total_tokens: number;
      cost: number;
      model: string;
    }
  /**
   * A background agent has been spawned. The UI shows a chip with elapsed
   * time until the matching `background_task_done` fires.
   */
  | {
      type: 'background_task_started';
      task_id: string;
      agent_name: string;
      description: string;
    }
  /** Background agent has finished (completed, failed, or cancelled). */
  | {
      type: 'background_task_done';
      task_id: string;
      status: 'completed' | 'failed' | 'cancelled';
    }
  | { type: 'done' }
  | { type: string; [k: string]: unknown };

// ---------------------------------------------------------------------------
// Unified timeline model
// ---------------------------------------------------------------------------

/** Semantic category of a timeline item — drives icon/color/layout. */
export type EventKind =
  | 'agent'
  | 'thinking'
  | 'tool_use'
  | 'tool_result'
  | 'error'
  /**
   * Live-only placeholder inserted right after the user submits a
   * message so the UI gives immediate feedback while the kernel is
   * setting up the turn / dispatching the LLM call. Cleared as soon as
   * the first real delta / tool call arrives, or when the turn ends.
   * Never appears in historical turn projections.
   */
  | 'in_progress'
  /**
   * Live-only multi-step plan progress widget. Created on the first
   * `plan_created` event and mutated in place by `plan_progress` /
   * `plan_replan` / `plan_completed`. Self-contained — renders its own
   * step list rather than going through the standard row layout.
   */
  | 'plan_card'
  /**
   * Live-only per-turn token usage footer. Rendered as a small muted
   * `↑12.5k ↓1.2k` line at the tail of the assistant turn. Populated
   * from the web channel's `usage` event (mirrors
   * `StreamEvent::TurnUsage`).
   */
  | 'token_footer'
  /**
   * Live-only chip list of currently-running background tasks. Inserted
   * on the first `background_task_started` for a turn, mutated in place
   * as tasks start/finish, and removed when the active set goes empty.
   */
  | 'background_tasks';

/** Per-step status for the live `plan_card` row. */
export type PlanStepUiStatus = 'pending' | 'running' | 'done' | 'failed' | 'needs_replan';

/** Single step within a plan card row. */
export interface PlanStep {
  /** 1-based step number for display (matches kernel's `current_step + 1`). */
  index: number;
  /** Free-form task description, parsed from the event's `status_text`. */
  task: string;
  status: PlanStepUiStatus;
  /** Failure / replan reason (only set when status is failed/needs_replan). */
  reason?: string | undefined;
}

/** Settled per-turn token totals for a `token_footer` row. */
export interface TurnUsage {
  /** Largest iteration's prompt size (kernel re-sends full context per iter). */
  input: number;
  /** Cumulative completion tokens across all iterations in the turn. */
  output: number;
}

/** One active background task for a `background_tasks` row. */
export interface BackgroundTaskInfo {
  taskId: string;
  name: string;
  description: string;
  /** `Date.now()` at insertion; used to compute the elapsed-time chip. */
  startedAt: number;
}

/** State for an in-flight `plan_card` timeline row. */
export interface PlanState {
  goal: string;
  totalSteps: number;
  steps: PlanStep[];
  /** 0-based index of the most recent step that received an update. */
  currentStepIdx: number | null;
  /** Optional reason from the most recent `plan_replan`. */
  replanReason?: string;
  /** Final summary set by `plan_completed`. */
  summary?: string;
  /** True once `plan_completed` fires. */
  completed: boolean;
}

/**
 * One renderable row in the session timeline.
 *
 * `seq` is monotonic **within its source** (historical or live). Keys for
 * React reconciliation should combine source + seq (e.g. `h-3`, `l-7`),
 * never compare seqs across sources.
 */
export interface TimelineItem {
  seq: number;
  /** 0-based turn index this item belongs to. */
  turn: number;
  kind: EventKind;
  /** Tool name for `tool_use` / `tool_result`. */
  tool?: string | undefined;
  /** Body for `agent` / `thinking` / `error`. */
  content?: string | undefined;
  /** Tool call arguments for `tool_use`. */
  input?: Record<string, unknown> | undefined;
  /** Tool result preview for `tool_result`. */
  output?: string | undefined;
  /** Tool execution duration (historical only). */
  durationMs?: number | undefined;
  /** Tool success flag (`tool_use` / `tool_result`). */
  success?: boolean | undefined;
  /** Still receiving WS deltas — callers may show a cursor/pulse. */
  streaming?: boolean | undefined;
  /** Plan widget state for `kind === "plan_card"`. */
  plan?: PlanState | undefined;
  /** Settled token totals for `kind === "token_footer"`. */
  usage?: TurnUsage | undefined;
  /** Active background tasks for `kind === "background_tasks"`. */
  bgTasks?: BackgroundTaskInfo[] | undefined;
}

/**
 * Flatten an array of `TurnTrace` into timeline items.
 *
 * Order within a turn: `thinking?` → per tool `[tool_use, tool_result|error]` → `agent?`.
 * A turn-level error is appended last.
 */
export function turnsToTimeline(turns: TurnTrace[]): TimelineItem[] {
  const items: TimelineItem[] = [];
  let seq = 0;

  turns.forEach((turn, turnIdx) => {
    for (const iter of turn.iterations) {
      if (iter.reasoning_text && iter.reasoning_text.trim().length > 0) {
        items.push({
          seq: seq++,
          turn: turnIdx,
          kind: 'thinking',
          content: iter.reasoning_text,
        });
      }

      for (const tc of iter.tool_calls) {
        items.push({
          seq: seq++,
          turn: turnIdx,
          kind: 'tool_use',
          tool: tc.name,
          input: tc.arguments,
          durationMs: tc.duration_ms,
          success: tc.success,
        });
        if (tc.error) {
          items.push({
            seq: seq++,
            turn: turnIdx,
            kind: 'error',
            tool: tc.name,
            content: tc.error,
          });
        } else if (tc.result_preview) {
          items.push({
            seq: seq++,
            turn: turnIdx,
            kind: 'tool_result',
            tool: tc.name,
            output: tc.result_preview,
            success: tc.success,
          });
        }
      }

      if (iter.text_preview && iter.text_preview.trim().length > 0) {
        items.push({
          seq: seq++,
          turn: turnIdx,
          kind: 'agent',
          content: iter.text_preview,
        });
      }
    }

    if (turn.error) {
      items.push({
        seq: seq++,
        turn: turnIdx,
        kind: 'error',
        content: turn.error,
      });
    }
  });

  return items;
}

// ---------------------------------------------------------------------------
// Session state helpers
// ---------------------------------------------------------------------------

/**
 * Concrete states produced by `rara_kernel::session::SessionState`.
 *
 * See `crates/kernel/src/session/mod.rs:243`. Sessions are **never
 * terminal** (`is_terminal()` always returns false), so `Completed`,
 * `Failed`, or `Cancelled` never appear — do not branch on them.
 */
export type KernelSessionState = 'Active' | 'Ready' | 'Suspended' | 'Paused';

/** UI grouping for the session list. */
export type SessionGroup = 'active' | 'dormant';

/**
 * Bucket a kernel session state into an IA group.
 *
 * - `active` — currently processing (`Active`) or idle-but-alive (`Ready`).
 * - `dormant` — temporarily offline (`Suspended` / `Paused`).
 */
export function sessionGroup(state: string): SessionGroup {
  switch (state.toLowerCase()) {
    case 'active':
    case 'ready':
      return 'active';
    default:
      return 'dormant';
  }
}

/**
 * Whether a session is expected to emit live stream events and should be
 * subscribed to via WebSocket.
 */
export function isLiveState(state: string | null): boolean {
  if (!state) return false;
  const s = state.toLowerCase();
  return s === 'active' || s === 'ready';
}

// ---------------------------------------------------------------------------
// Cascade execution trace
// ---------------------------------------------------------------------------
// Mirrors `rara_kernel::cascade::CascadeTrace` (crates/kernel/src/cascade.rs).
// Returned by `GET /api/v1/chat/sessions/{key}/trace?seq={seq}` and rendered
// by `<CascadeModal>` when the user expands an assistant turn's "📊 详情"
// button. Empty traces (no recorded ticks/entries) are surfaced explicitly so
// the modal can show an empty state rather than a broken view.

/** Classification of a single entry within a cascade tick. */
export type CascadeEntryKind = 'user_input' | 'thought' | 'action' | 'observation';

/** A single entry: user input, assistant thought, tool action, or observation. */
export interface CascadeEntry {
  /** Stable, human-readable identifier (`"{prefix}.{tick}-{short_id}-{seq}"`). */
  id: string;
  kind: CascadeEntryKind;
  /** Display content — text, JSON-encoded tool args, or tool output. */
  content: string;
  /** RFC3339 timestamp of the underlying tape entry. */
  timestamp: string;
  /** Optional structured metadata copied from the tape entry. */
  metadata?: unknown;
}

/** One reasoning-action cycle (an LLM call + its emitted entries). */
export interface CascadeTick {
  /** Zero-based tick index within the trace. */
  index: number;
  entries: CascadeEntry[];
}

/** High-level summary statistics for a cascade trace. */
export interface CascadeSummary {
  tick_count: number;
  tool_call_count: number;
  total_entries: number;
}

/** Top-level cascade trace for a single agent turn. */
export interface CascadeTrace {
  /** Opaque identifier — typically `"{session_key}-{seq}"`. */
  message_id: string;
  ticks: CascadeTick[];
  summary: CascadeSummary;
}

// ---------------------------------------------------------------------------
// Execution trace
// ---------------------------------------------------------------------------
// Mirrors `rara_kernel::trace::ExecutionTrace` (crates/kernel/src/trace.rs).
// Returned by `GET /api/v1/chat/sessions/{key}/execution-trace?seq={seq}` and
// rendered by `<ExecutionTraceModal>` — the per-turn rationale / thinking /
// plan / tools / usage summary, matching the Telegram "📊 详情" payload.

/** Record of a single tool invocation within a turn. */
export interface ToolTraceEntry {
  name: string;
  /** Duration in milliseconds, when the invocation completed. */
  duration_ms: number | null;
  success: boolean;
  summary: string;
  error: string | null;
}

/** Summary of a single agent turn execution. */
export interface ExecutionTrace {
  duration_secs: number;
  iterations: number;
  model: string;
  input_tokens: number;
  output_tokens: number;
  thinking_ms: number;
  /** Truncated reasoning text (first ~500 chars). */
  thinking_preview: string;
  /** Plan steps with status. */
  plan_steps: string[];
  /** High-level rationale the LLM stated for this turn, when any. */
  turn_rationale?: string;
  tools: ToolTraceEntry[];
  /** Rara internal message ID for end-to-end correlation. */
  rara_message_id: string;
}
