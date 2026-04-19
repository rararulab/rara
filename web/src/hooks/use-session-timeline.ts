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

import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/api/client";
import {
  isLiveState,
  turnsToTimeline,
  type PlanState,
  type PlanStep,
  type PlanStepStatusEvent,
  type PlanStepUiStatus,
  type StreamEvent,
  type TimelineItem,
  type TurnTrace,
} from "@/api/kernel-types";
import { randomLoadingHint } from "./loading-hints";

const TURNS_REFETCH_MS = 5_000;

/** Result returned by {@link useSessionTimeline}. */
export interface SessionTimelineState {
  /** Merged timeline: historical turns followed by in-flight live events. */
  items: TimelineItem[];
  /** Items projected from completed historical turns. */
  historicalItems: TimelineItem[];
  /** Items from the in-flight WS stream (empty when not streaming). */
  liveItems: TimelineItem[];
  /** True while a WebSocket stream is actively receiving events. */
  isStreaming: boolean;
  /** Raw turns for callers that need turn-level metadata (duration, model). */
  turns: TurnTrace[];
  isLoading: boolean;
  isError: boolean;
  refetch: () => void;
}

/**
 * Unified session execution timeline — merges historical turns from
 * `/kernel/sessions/{key}/turns` with live WS events into a single
 * ordered `TimelineItem[]`.
 *
 * The caller is agnostic to data source: historical and live items are
 * indistinguishable beyond the `streaming` flag on items still receiving
 * deltas. When the backend's `done` event fires, live items are cleared;
 * the next turns-query refetch will surface them as historical items.
 */
export function useSessionTimeline(
  sessionKey: string | null,
  sessionState: string | null,
  /** When false, disables the 5s turns polling (respects Auto-refresh). */
  autoRefresh = true,
): SessionTimelineState {
  const turnsQuery = useQuery({
    queryKey: ["session-turns", sessionKey],
    queryFn: () =>
      api.get<TurnTrace[]>(`/api/v1/kernel/sessions/${sessionKey}/turns`),
    enabled: !!sessionKey,
    refetchInterval: autoRefresh ? TURNS_REFETCH_MS : false,
  });

  const turns = useMemo(() => turnsQuery.data ?? [], [turnsQuery.data]);
  const historicalItems = useMemo(() => turnsToTimeline(turns), [turns]);

  const [liveItems, setLiveItems] = useState<TimelineItem[]>([]);
  const [isStreaming, setIsStreaming] = useState(false);

  // Read through refs so the WS effect does not re-subscribe every time
  // the 5s turns refetch mutates turn count or query identity.
  const turnsLenRef = useRef(0);
  turnsLenRef.current = turns.length;

  const refetchTurnsRef = useRef(turnsQuery.refetch);
  refetchTurnsRef.current = turnsQuery.refetch;

  // Depending only on (sessionKey, sessionState) prevents the WS from
  // reconnecting whenever the 5s turns refetch mutates historical data.
  useEffect(() => {
    if (!sessionKey || !isLiveState(sessionState)) {
      setLiveItems([]);
      setIsStreaming(false);
      return;
    }

    const host = window.location.host;
    const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
    const token = localStorage.getItem("access_token") ?? "";
    const ws = new WebSocket(
      `${protocol}//${host}/api/v1/kernel/sessions/${sessionKey}/stream?token=${token}`,
    );

    // Live seq is an independent monotonic counter; combine with the "l-"
    // prefix on React keys to avoid any collision with historical seqs.
    let liveSeq = 0;
    const nextSeq = () => liveSeq++;

    // Track the in-flight text/thinking item's seq so deltas can be appended
    // in place, and the tool_use seq per tool id so tool_call_end finalizes
    // the correct row.
    let currentTextSeq: number | null = null;
    let currentThinkSeq: number | null = null;
    const toolSeqById = new Map<string, number>();

    // The active plan widget for this turn. A single `plan_card` row per
    // plan; subsequent `plan_progress` / `plan_replan` / `plan_completed`
    // events mutate the same row in place.
    let planSeq: number | null = null;

    // Placeholder "thinking…" row inserted as soon as the WS opens so the
    // user gets immediate feedback while the kernel is bootstrapping the
    // turn (LLM dispatch can take 2-30s for cold runs). Cleared on the
    // first real delta/tool call, on `done`, or when the WS closes.
    let placeholderSeq: number | null = null;
    const clearPlaceholder = () => {
      if (placeholderSeq === null) return;
      const target = placeholderSeq;
      placeholderSeq = null;
      setLiveItems((prev) =>
        prev.filter((it) => !(it.seq === target && it.kind === "in_progress")),
      );
    };

    // Live events belong to the turn after the last one already recorded.
    // Captured at WS-open time; not refreshed mid-stream. `done` clears
    // live state, so drift between this value and turnsQuery is bounded.
    const liveTurnIdx = turnsLenRef.current;

    ws.onopen = () => {
      setIsStreaming(true);
      const seq = nextSeq();
      placeholderSeq = seq;
      setLiveItems([
        {
          seq,
          turn: liveTurnIdx,
          kind: "in_progress",
          content: randomLoadingHint(),
          streaming: true,
        },
      ]);
    };

    ws.onmessage = (ev) => {
      let event: StreamEvent;
      try {
        event = JSON.parse(ev.data) as StreamEvent;
      } catch {
        return;
      }

      switch (event.type) {
        case "text_delta": {
          const delta = (event as { text?: string }).text ?? "";
          if (!delta) break;
          clearPlaceholder();
          setLiveItems((prev) => {
            if (currentTextSeq !== null) {
              const target = currentTextSeq;
              return prev.map((it) =>
                it.seq === target && it.kind === "agent"
                  ? { ...it, content: (it.content ?? "") + delta }
                  : it,
              );
            }
            const seq = nextSeq();
            currentTextSeq = seq;
            return [
              ...prev,
              {
                seq,
                turn: liveTurnIdx,
                kind: "agent",
                content: delta,
                streaming: true,
              },
            ];
          });
          break;
        }

        case "reasoning_delta": {
          const delta = (event as { text?: string }).text ?? "";
          if (!delta) break;
          clearPlaceholder();
          setLiveItems((prev) => {
            if (currentThinkSeq !== null) {
              const target = currentThinkSeq;
              return prev.map((it) =>
                it.seq === target && it.kind === "thinking"
                  ? { ...it, content: (it.content ?? "") + delta }
                  : it,
              );
            }
            const seq = nextSeq();
            currentThinkSeq = seq;
            return [
              ...prev,
              {
                seq,
                turn: liveTurnIdx,
                kind: "thinking",
                content: delta,
                streaming: true,
              },
            ];
          });
          break;
        }

        case "text_clear": {
          // Kernel emits text_clear before tool_call_start to discard
          // intermediate narration. Remove the in-flight agent row.
          if (currentTextSeq !== null) {
            const target = currentTextSeq;
            setLiveItems((prev) =>
              prev.filter((it) => !(it.seq === target && it.kind === "agent")),
            );
            currentTextSeq = null;
          }
          break;
        }

        case "tool_call_start": {
          clearPlaceholder();
          const e = event as {
            name: string;
            id: string;
            arguments: Record<string, unknown>;
          };
          const closedTextSeq = currentTextSeq;
          const closedThinkSeq = currentThinkSeq;
          currentTextSeq = null;
          currentThinkSeq = null;

          const seq = nextSeq();
          toolSeqById.set(e.id, seq);

          setLiveItems((prev) => {
            const finalized = prev.map((it) => {
              if (
                (closedTextSeq !== null && it.seq === closedTextSeq) ||
                (closedThinkSeq !== null && it.seq === closedThinkSeq)
              ) {
                return { ...it, streaming: false };
              }
              return it;
            });
            return [
              ...finalized,
              {
                seq,
                turn: liveTurnIdx,
                kind: "tool_use",
                tool: e.name,
                input: e.arguments,
                streaming: true,
              },
            ];
          });
          break;
        }

        case "tool_call_end": {
          const e = event as {
            id: string;
            result_preview: string;
            success: boolean;
            error: string | null;
          };
          const usedSeq = toolSeqById.get(e.id);
          toolSeqById.delete(e.id);

          setLiveItems((prev) => {
            const finalized =
              usedSeq !== undefined
                ? prev.map((it) =>
                    it.seq === usedSeq && it.kind === "tool_use"
                      ? { ...it, streaming: false, success: e.success }
                      : it,
                  )
                : prev;

            if (e.error) {
              return [
                ...finalized,
                {
                  seq: nextSeq(),
                  turn: liveTurnIdx,
                  kind: "error",
                  content: e.error,
                },
              ];
            }
            if (e.result_preview) {
              return [
                ...finalized,
                {
                  seq: nextSeq(),
                  turn: liveTurnIdx,
                  kind: "tool_result",
                  output: e.result_preview,
                  success: e.success,
                },
              ];
            }
            return finalized;
          });
          break;
        }

        case "turn_metrics": {
          // Turn boundary — flush streaming flags; keep items so the
          // just-completed turn stays visible until turnsQuery refetches.
          currentTextSeq = null;
          currentThinkSeq = null;
          setLiveItems((prev) =>
            prev.map((it) => ({ ...it, streaming: false })),
          );
          break;
        }

        case "plan_created": {
          clearPlaceholder();
          const e = event as Extract<StreamEvent, { type: "plan_created" }>;
          const steps: PlanStep[] = Array.from(
            { length: e.total_steps },
            (_, i) => ({ index: i + 1, task: "", status: "pending" }),
          );
          const plan: PlanState = {
            goal: e.goal,
            totalSteps: e.total_steps,
            steps,
            currentStepIdx: null,
            completed: false,
          };
          const seq = nextSeq();
          planSeq = seq;
          setLiveItems((prev) => [
            ...prev,
            {
              seq,
              turn: liveTurnIdx,
              kind: "plan_card",
              plan,
              streaming: true,
            },
          ]);
          break;
        }

        case "plan_progress": {
          clearPlaceholder();
          if (planSeq === null) break;
          const e = event as Extract<StreamEvent, { type: "plan_progress" }>;
          const target = planSeq;
          setLiveItems((prev) =>
            prev.map((it) => {
              if (it.seq !== target || it.kind !== "plan_card" || !it.plan) {
                return it;
              }
              return { ...it, plan: applyPlanProgress(it.plan, e) };
            }),
          );
          break;
        }

        case "plan_replan": {
          if (planSeq === null) break;
          const e = event as Extract<StreamEvent, { type: "plan_replan" }>;
          const target = planSeq;
          setLiveItems((prev) =>
            prev.map((it) => {
              if (it.seq !== target || it.kind !== "plan_card" || !it.plan) {
                return it;
              }
              return { ...it, plan: applyPlanReplan(it.plan, e.reason) };
            }),
          );
          break;
        }

        case "plan_completed": {
          if (planSeq === null) break;
          const e = event as Extract<StreamEvent, { type: "plan_completed" }>;
          const target = planSeq;
          planSeq = null;
          setLiveItems((prev) =>
            prev.map((it) => {
              if (it.seq !== target || it.kind !== "plan_card" || !it.plan) {
                return it;
              }
              return {
                ...it,
                streaming: false,
                plan: { ...it.plan, summary: e.summary, completed: true },
              };
            }),
          );
          break;
        }

        case "done":
          setIsStreaming(false);
          // Drop the placeholder unconditionally — turns that produced
          // zero deltas (e.g. a tool-only turn that errored before any
          // streaming) would otherwise leave the spinner row hanging
          // until the historical refetch overwrote `liveItems`.
          clearPlaceholder();
          // Trigger an immediate refetch so the just-completed turn
          // appears as historical items before we clear live rows.
          // This avoids a 5s blank/stale window after every turn.
          refetchTurnsRef.current().then(() => setLiveItems([]));
          break;

        default:
          // Unknown / unhandled event types (UsageUpdate,
          // BackgroundTaskStarted, etc.) are ignored. Add cases here as
          // UI coverage expands.
          break;
      }
    };

    ws.onclose = () => {
      setIsStreaming(false);
      clearPlaceholder();
    };
    ws.onerror = () => {
      setIsStreaming(false);
      clearPlaceholder();
    };

    return () => {
      ws.close();
      setIsStreaming(false);
      setLiveItems([]);
    };
  }, [sessionKey, sessionState]);

  const items = useMemo(
    () => [...historicalItems, ...liveItems],
    [historicalItems, liveItems],
  );

  return {
    items,
    historicalItems,
    liveItems,
    isStreaming,
    turns,
    isLoading: turnsQuery.isLoading,
    isError: turnsQuery.isError,
    refetch: turnsQuery.refetch,
  };
}

/** Map kernel `PlanStepStatus` JSON onto the UI-facing status enum. */
function mapStepStatus(status: PlanStepStatusEvent): {
  ui: PlanStepUiStatus;
  reason?: string;
} {
  if (status === "running") return { ui: "running" };
  if (status === "done") return { ui: "done" };
  if ("failed" in status) {
    return { ui: "failed", reason: status.failed.reason };
  }
  return { ui: "needs_replan", reason: status.needs_replan.reason };
}

/**
 * Extract the task description from kernel `status_text`.
 *
 * Kernel formats status_text as `第N步：{task}…`; the TG adapter splits
 * on the full-width colon (U+FF1A) and trims the trailing ellipsis. We
 * mirror that logic so step rows render the same task text across
 * channels.
 */
function extractTask(statusText: string): string {
  const colon = statusText.indexOf("\uFF1A");
  const tail = colon >= 0 ? statusText.slice(colon + 1) : statusText;
  return tail.replace(/\u2026$/, "").trim();
}

/** Apply a `plan_progress` event to the current plan state. */
function applyPlanProgress(
  plan: PlanState,
  event: Extract<StreamEvent, { type: "plan_progress" }>,
): PlanState {
  const { current_step: idx, total_steps: total, step_status, status_text } =
    event;
  const { ui, reason } = mapStepStatus(step_status);

  const steps = plan.steps.slice();
  // Replan can dynamically extend the plan beyond the original length.
  while (steps.length <= idx) {
    steps.push({ index: steps.length + 1, task: "", status: "pending" });
  }
  // When the active step advances, finalize any prior `running` step that
  // never received an explicit `done` (kernel emits Done for the prior
  // step before Running for the next, but be defensive against drops).
  if (plan.currentStepIdx !== null && plan.currentStepIdx !== idx) {
    const prev = steps[plan.currentStepIdx];
    if (prev && prev.status === "running") {
      steps[plan.currentStepIdx] = { ...prev, status: "done" };
    }
  }

  const existing = steps[idx]!;
  const task = existing.task || extractTask(status_text);
  steps[idx] = { ...existing, task, status: ui, reason };

  return {
    ...plan,
    totalSteps: Math.max(plan.totalSteps, total, steps.length),
    steps,
    currentStepIdx: idx,
  };
}

/**
 * Apply a `plan_replan` event: mark the current step failed and drop
 * trailing pending steps. Replacement steps arrive via subsequent
 * `plan_progress` events at higher indices and dynamically extend the
 * plan in {@link applyPlanProgress}.
 */
function applyPlanReplan(plan: PlanState, reason: string): PlanState {
  const steps = plan.steps.slice();
  if (plan.currentStepIdx !== null) {
    const cur = steps[plan.currentStepIdx];
    if (cur) {
      steps[plan.currentStepIdx] = {
        ...cur,
        status: "needs_replan",
        reason,
      };
    }
  }
  const trimmed = steps.filter((s) => s.status !== "pending");
  return { ...plan, steps: trimmed, replanReason: reason };
}
