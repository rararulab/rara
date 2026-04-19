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
 * Narrow subset of kernel `StreamEvent` the UI consumes today.
 *
 * The kernel emits more variants (PlanCreated, UsageUpdate,
 * BackgroundTaskStarted, etc.); we parse defensively — unknown variants
 * are ignored by the hook.
 */
export type StreamEvent =
  | { type: "text_delta"; text: string }
  | { type: "reasoning_delta"; text: string }
  | {
      type: "tool_call_start";
      name: string;
      id: string;
      arguments: Record<string, unknown>;
    }
  | {
      type: "tool_call_end";
      id: string;
      result_preview: string;
      success: boolean;
      error: string | null;
    }
  | {
      type: "turn_metrics";
      duration_ms: number;
      iterations: number;
      tool_calls: number;
      model: string;
      rara_message_id: string;
    }
  | { type: "done" }
  | { type: string; [k: string]: unknown };

// ---------------------------------------------------------------------------
// Unified timeline model
// ---------------------------------------------------------------------------

/** Semantic category of a timeline item — drives icon/color/layout. */
export type EventKind =
  | "agent"
  | "thinking"
  | "tool_use"
  | "tool_result"
  | "error"
  /**
   * Live-only placeholder inserted right after the user submits a
   * message so the UI gives immediate feedback while the kernel is
   * setting up the turn / dispatching the LLM call. Cleared as soon as
   * the first real delta / tool call arrives, or when the turn ends.
   * Never appears in historical turn projections.
   */
  | "in_progress";

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
  tool?: string;
  /** Body for `agent` / `thinking` / `error`. */
  content?: string;
  /** Tool call arguments for `tool_use`. */
  input?: Record<string, unknown>;
  /** Tool result preview for `tool_result`. */
  output?: string;
  /** Tool execution duration (historical only). */
  durationMs?: number;
  /** Tool success flag (`tool_use` / `tool_result`). */
  success?: boolean;
  /** Still receiving WS deltas — callers may show a cursor/pulse. */
  streaming?: boolean;
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
          kind: "thinking",
          content: iter.reasoning_text,
        });
      }

      for (const tc of iter.tool_calls) {
        items.push({
          seq: seq++,
          turn: turnIdx,
          kind: "tool_use",
          tool: tc.name,
          input: tc.arguments,
          durationMs: tc.duration_ms,
          success: tc.success,
        });
        if (tc.error) {
          items.push({
            seq: seq++,
            turn: turnIdx,
            kind: "error",
            tool: tc.name,
            content: tc.error,
          });
        } else if (tc.result_preview) {
          items.push({
            seq: seq++,
            turn: turnIdx,
            kind: "tool_result",
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
          kind: "agent",
          content: iter.text_preview,
        });
      }
    }

    if (turn.error) {
      items.push({
        seq: seq++,
        turn: turnIdx,
        kind: "error",
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
export type KernelSessionState = "Active" | "Ready" | "Suspended" | "Paused";

/** UI grouping for the session list. */
export type SessionGroup = "active" | "dormant";

/**
 * Bucket a kernel session state into an IA group.
 *
 * - `active` — currently processing (`Active`) or idle-but-alive (`Ready`).
 * - `dormant` — temporarily offline (`Suspended` / `Paused`).
 */
export function sessionGroup(state: string): SessionGroup {
  switch (state.toLowerCase()) {
    case "active":
    case "ready":
      return "active";
    default:
      return "dormant";
  }
}

/**
 * Whether a session is expected to emit live stream events and should be
 * subscribed to via WebSocket.
 */
export function isLiveState(state: string | null): boolean {
  if (!state) return false;
  const s = state.toLowerCase();
  return s === "active" || s === "ready";
}
