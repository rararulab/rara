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

import { Bookmark } from 'lucide-react';
import { useCallback } from 'react';

import type { SessionAnchor } from '@/api/types';

/** Maximum number of characters to render from an anchor's name before
 *  truncating with an ellipsis. Names like `daily-summary-2026-04-28`
 *  push past this; the full name lives in the marker's `title`
 *  attribute so hover still surfaces it. */
const ANCHOR_NAME_MAX = 18;

/** Visible label for an anchor, truncated to keep the strip compact. */
function truncateName(name: string): string {
  if (name.length <= ANCHOR_NAME_MAX) return name;
  return `${name.slice(0, ANCHOR_NAME_MAX - 1)}…`;
}

export interface TimelineChapterStripProps {
  /** Anchors for this session in append order. The strip renders one
   *  marker per entry; an empty array renders nothing (no header, no
   *  empty-state — the topology page has its own empty state). */
  anchors: SessionAnchor[];
  /** Invoked with the clicked anchor and its successor (or `null` when
   *  the clicked anchor is the most recent). The parent decides what to
   *  do with the segment — typically calls
   *  {@link fetchSessionMessagesBetweenAnchors} and passes the result
   *  back into `TimelineView`. */
  onSelectAnchor: (from: SessionAnchor, to: SessionAnchor | null) => void;
}

/**
 * Compact chapter-marker strip for a session's tape. One clickable
 * marker per anchor, in append order, showing the anchor name (truncated
 * if long) and `entry_count_in_segment` as a small badge. Mounted as a
 * sibling of `TimelineView` on the topology page so the timeline pane
 * itself stays focused on rendering messages.
 *
 * Naming: "chapter strip" rather than "timeline strip" to avoid
 * colliding with `TimelineView` (the chat-history pane) and
 * `TapeLineageView` (the fork-tree visualization that already speaks
 * "timeline"). Both are established in this codebase; introducing a
 * third would muddy the surface.
 */
export function TimelineChapterStrip({ anchors, onSelectAnchor }: TimelineChapterStripProps) {
  const handleClick = useCallback(
    (index: number) => {
      const from = anchors[index];
      // Successor lookup defines the half-open `[from, to)` segment.
      // The most-recent anchor has no successor → caller passes
      // `null`, which translates to "no `to_anchor` query param" in
      // the API helper → backend reads to EOF.
      const to = index + 1 < anchors.length ? (anchors[index + 1] ?? null) : null;
      if (!from) return;
      onSelectAnchor(from, to);
    },
    [anchors, onSelectAnchor],
  );

  if (anchors.length === 0) return null;

  return (
    <div
      data-testid="chapter-strip"
      role="list"
      aria-label="Session anchors"
      className="flex flex-wrap items-center gap-1.5 border-b border-border px-2 py-1.5"
    >
      {anchors.map((anchor, index) => (
        // Wrap the button in an explicit `listitem`-role span so the
        // strip's `role="list"` still describes a real list at the AT
        // layer (a list whose only children are buttons fails the WAI
        // role-children rule). Keeping a separate wrapper avoids the
        // jsx-a11y `interactive-element-to-noninteractive-role` lint
        // that fires when role="listitem" lands directly on a button.
        <span key={anchor.anchor_id} role="listitem">
          <button
            type="button"
            data-testid="chapter-marker"
            data-anchor-id={anchor.anchor_id}
            title={anchor.name}
            onClick={() => handleClick(index)}
            className="inline-flex items-center gap-1 rounded border border-border bg-background px-1.5 py-0.5 text-[11px] text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
          >
            <Bookmark className="h-3 w-3" aria-hidden="true" />
            <span className="font-mono">{truncateName(anchor.name)}</span>
            <span
              data-testid="chapter-marker-count"
              className="ml-0.5 rounded bg-muted px-1 text-[10px] tabular-nums text-foreground"
            >
              {anchor.entry_count_in_segment}
            </span>
          </button>
        </span>
      ))}
    </div>
  );
}
