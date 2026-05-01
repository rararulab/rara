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

import { useMemo } from 'react';

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
 * Many vendor callbacks (`onAcceptPlan`, `onAddAnnotation`, `onBranch`,
 * `onOpenDetails`, …) are intentionally left undefined; rara doesn't yet
 * expose those surfaces. The vendor handles `undefined` gracefully — any
 * crash on a missing optional is a vendor bug to flag, not something to
 * silently work around.
 */
export interface RaraTurnCardProps {
  turn: TurnCardData;
}

export function RaraTurnCard({ turn }: RaraTurnCardProps) {
  const activities = useMemo<ActivityItem[]>(() => {
    const items: ActivityItem[] = [];
    // Vendor sorts activities by `timestamp` — only relative ordering
    // matters within a single turn, so a small monotonic counter anchored
    // at the turn's `createdAt` (or 0 for live turns) gives the vendor
    // stable, deterministic ordering without calling `Date.now()` during
    // render.
    let cursor = turn.createdAt ?? 0;

    if (turn.reasoning.trim().length > 0) {
      items.push({
        id: `${turn.id}:thinking`,
        type: 'thinking',
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
  }, [turn.id, turn.createdAt, turn.reasoning, turn.toolCalls, turn.inFlight]);

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

  return (
    <div className="space-y-2">
      <VendorTurnCard
        turnId={turn.id}
        activities={activities}
        {...responseProps}
        isStreaming={turn.inFlight}
        isComplete={!turn.inFlight}
      />
      {turn.markers.length > 0 && (
        <div className="space-y-1.5">
          {turn.markers.map((marker, idx) => (
            <SpawnMarker key={`${turn.id}-marker-${String(idx)}`} marker={marker} />
          ))}
        </div>
      )}
    </div>
  );
}
