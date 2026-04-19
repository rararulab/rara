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

import { KIND_PALETTE, eventLabel } from './timeline-colors';

import type { EventKind, TimelineItem } from '@/api/kernel-types';
import { cn } from '@/lib/utils';

interface Segment {
  startIdx: number;
  endIdx: number;
  kind: EventKind;
  count: number;
}

/**
 * Merge adjacent items of the same kind into display segments.
 *
 * Turn boundaries force a break even when the same color continues, so
 * the rhythm of each turn stays visually separable.
 */
function buildSegments(items: TimelineItem[]): Segment[] {
  const segments: Segment[] = [];
  let start = 0;
  for (let i = 0; i < items.length; i++) {
    const prev = items[i - 1];
    const curr = items[i];
    if (!curr) continue;
    const boundary = !prev || prev.kind !== curr.kind || prev.turn !== curr.turn;
    if (boundary && i !== 0) {
      const head = items[start];
      if (head) {
        segments.push({
          startIdx: start,
          endIdx: i - 1,
          kind: head.kind,
          count: i - start,
        });
      }
      start = i;
    }
  }
  if (items.length > 0) {
    const head = items[start];
    if (head) {
      segments.push({
        startIdx: start,
        endIdx: items.length - 1,
        kind: head.kind,
        count: items.length - start,
      });
    }
  }
  return segments;
}

export interface TimelineBarProps {
  items: TimelineItem[];
  /** Currently selected item index (highlights the containing segment). */
  selectedIdx: number | null;
  /** Click handler: receives the segment's first item index. */
  onSegmentClick?: (idx: number) => void;
}

/**
 * Horizontal color-coded rhythm bar.
 *
 * Each segment represents a run of adjacent items sharing the same
 * {@link EventKind}; width is proportional to the count. Clicking a
 * segment selects its first item.
 */
export function TimelineBar({ items, selectedIdx, onSegmentClick }: TimelineBarProps) {
  const segments = useMemo(() => buildSegments(items), [items]);

  if (segments.length === 0) return null;

  const total = items.length;

  return (
    <div
      className="flex h-5 gap-0.5 overflow-hidden rounded"
      role="navigation"
      aria-label="Execution timeline"
    >
      {segments.map((seg, segIdx) => {
        const isSelected =
          selectedIdx !== null && selectedIdx >= seg.startIdx && selectedIdx <= seg.endIdx;
        const palette = KIND_PALETTE[seg.kind];
        const head = items[seg.startIdx];
        const widthPct = Math.max((seg.count / total) * 100, 0.5);
        const label = eventLabel(seg.kind, head?.tool);

        return (
          <button
            key={segIdx}
            type="button"
            onClick={() => onSegmentClick?.(seg.startIdx)}
            className={cn(
              'group relative h-full min-w-[4px] transition-all duration-150 hover:opacity-80',
              isSelected ? palette.barActive : palette.bar,
            )}
            style={{ width: `${widthPct}%` }}
            title={seg.count > 1 ? `${label} (+${seg.count - 1} more)` : label}
          >
            <span className="pointer-events-none absolute bottom-full left-1/2 z-10 mb-1 hidden -translate-x-1/2 group-hover:block">
              <span className="block whitespace-nowrap rounded border bg-popover px-2 py-1 text-[10px] text-popover-foreground shadow-md">
                {label}
                {seg.count > 1 && (
                  <span className="ml-1 text-muted-foreground">+{seg.count - 1}</span>
                )}
              </span>
            </span>
          </button>
        );
      })}
    </div>
  );
}
