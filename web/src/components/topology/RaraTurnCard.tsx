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

import { useMemo, useState } from 'react';

import { CascadeModal } from './CascadeModal';
import { ExecutionTraceModal } from './ExecutionTraceModal';
import { SpawnMarker } from './SpawnMarker';
import type { TurnCardData } from './TurnCard';

import { TurnCard as VendorTurnCard } from '~vendor/components/chat/TurnCard';
import type { ActivityItem, ResponseContent } from '~vendor/components/chat/TurnCard';

/**
 * Rara-side adapter that maps the in-tree `TurnCardData` shape onto the
 * vendor `TurnCard` (craft-agents-oss). Rara accumulates assistant text,
 * reasoning, and tool calls through its own reducers (see `TurnCard.tsx`'s
 * `buildTurnsFromHistory` / `buildTurnsFromEvents`); the vendor expects a
 * flat `ActivityItem[]` plus a single `ResponseContent`. This file is the
 * only place that bridges the two — keeping the rest of the topology tree
 * unaware of the vendor surface.
 *
 * The trace + cascade affordances are wired through the vendor's
 * `onOpenDetails` (three-dot actions menu → "view turn details") and
 * `onOpenActivityDetails` (clicking a completed tool row) slots. Both
 * slots are left undefined when `turn.finalSeq === null` or
 * `turn.inFlight === true`, which causes the vendor to suppress the
 * three-dot trigger entirely (see `TurnCardActionsMenu.tsx` lines 39-42)
 * and the activity row's hover-affordance (see `TurnCard.tsx` ~line 1030).
 * That's the structural mitigation for #1672 — affordances cannot leak
 * onto live or seq-less turns because the props they depend on are not
 * passed.
 */
export interface RaraTurnCardProps {
  turn: TurnCardData;
  /** Session key whose trace endpoints back the modals. */
  sessionKey: string;
}

export function RaraTurnCard({ turn, sessionKey }: RaraTurnCardProps) {
  const [traceOpen, setTraceOpen] = useState(false);
  const [cascadeOpen, setCascadeOpen] = useState(false);

  const activities = useMemo<ActivityItem[]>(() => {
    const items: ActivityItem[] = [];
    // Vendor sorts activities by `timestamp` — only relative ordering
    // matters within a single turn, so a small monotonic counter anchored
    // at the turn's `createdAt` (or 0 for live turns) gives the vendor
    // stable, deterministic ordering without calling `Date.now()` during
    // render.
    let cursor = turn.createdAt ?? 0;

    if (turn.reasoning.trim().length > 0) {
      // When the turn has reasoning but no final text and no tool calls,
      // emit the reasoning as `type: 'intermediate'` instead of
      // `type: 'thinking'`. The vendor's `hasNoMeaningfulWork` gate
      // (TurnCard.tsx ~line 2884) treats `thinking` as no-meaningful-work
      // and suppresses the entire card via `return null`, swallowing the
      // reasoning trace silently. `intermediate` with non-empty content
      // is counted as meaningful, so the card renders. See
      // `specs/issue-2031-thinking-only-turn-render.spec.md` for the
      // failure mode this rerouting fixes.
      const thinkingOnly = turn.text.length === 0 && turn.toolCalls.length === 0;
      items.push({
        id: `${turn.id}:thinking`,
        type: thinkingOnly ? 'intermediate' : 'thinking',
        status: turn.inFlight ? 'running' : 'completed',
        content: turn.reasoning,
        timestamp: cursor++,
      });
    }

    for (const call of turn.toolCalls) {
      const failed = call.result !== null && !call.result.success;
      const pending = call.result === null;
      // `exactOptionalPropertyTypes` rejects assigning `undefined` to an
      // optional field — only add `content` / `error` when we actually
      // have a value.
      const item: ActivityItem = {
        id: call.id,
        type: 'tool',
        status: pending ? 'running' : failed ? 'error' : 'completed',
        toolName: call.name,
        toolUseId: call.id,
        timestamp: cursor++,
      };
      if (call.result?.preview) item.content = call.result.preview;
      if (failed && call.result?.error) item.error = call.result.error;
      items.push(item);
    }

    return items;
  }, [turn.id, turn.createdAt, turn.reasoning, turn.toolCalls, turn.inFlight, turn.text]);

  const response = useMemo<ResponseContent | undefined>(() => {
    if (turn.text.length === 0) return undefined;
    return {
      text: turn.text,
      isStreaming: turn.inFlight,
      isPlan: false,
    };
  }, [turn.text, turn.inFlight]);

  // Conditionally spread `response` only when defined — vendor's
  // `TurnCardProps.response` is optional, but `exactOptionalPropertyTypes`
  // forbids passing `undefined` explicitly.
  const responseProps = response ? { response } : {};

  // Affordance gate: the trace / cascade endpoints are keyed on a
  // persisted seq, so leave the slots undefined for live turns and for
  // any turn whose seq we have not yet observed.
  const inspectable = turn.finalSeq !== null && !turn.inFlight;
  const inspectProps = inspectable
    ? {
        onOpenDetails: () => setTraceOpen(true),
        // Cascade is per-turn, so any activity row opens the same modal.
        // The vendor passes the activity to the callback; we ignore it.
        onOpenActivityDetails: () => setCascadeOpen(true),
      }
    : {};

  // Mirror the vendor's suppression rules so the `data-turn-id` wrapper
  // is absent for turns the vendor would render as `null`. The vendor
  // returns null when (activities.length === 0 && !response && isComplete)
  // OR when every activity is non-meaningful work (TurnCard.tsx
  // ~line 2873-2898). For the adapter we only need the no-content case —
  // the meaningful-work check is satisfied by the `intermediate` reroute
  // above whenever reasoning is non-empty.
  const hasContent = activities.length > 0 || response !== undefined;
  if (!hasContent) {
    return null;
  }

  return (
    <div className="space-y-2" data-turn-id={turn.id}>
      <VendorTurnCard
        turnId={turn.id}
        activities={activities}
        {...responseProps}
        isStreaming={turn.inFlight}
        isComplete={!turn.inFlight}
        {...inspectProps}
      />
      {turn.markers.length > 0 && (
        <div className="space-y-1.5">
          {turn.markers.map((marker, idx) => (
            <SpawnMarker key={`${turn.id}-marker-${String(idx)}`} marker={marker} />
          ))}
        </div>
      )}
      {inspectable && (
        <>
          <ExecutionTraceModal
            sessionKey={sessionKey}
            seq={turn.finalSeq}
            open={traceOpen}
            onOpenChange={setTraceOpen}
          />
          <CascadeModal
            sessionKey={sessionKey}
            seq={turn.finalSeq}
            open={cascadeOpen}
            onOpenChange={setCascadeOpen}
          />
        </>
      )}
    </div>
  );
}
