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
import { useCallback, useMemo } from 'react';

import type { SessionAnchor } from '@/api/types';

/** Maximum characters of an anchor's name to render before truncation.
 *  The full name is preserved on the row's `title` attribute so hover
 *  surfaces it. The minimap is denser vertically than the old horizontal
 *  strip, so we can afford a slightly longer visible label. */
const ANCHOR_NAME_MAX = 24;

/** Visible label for an anchor, truncated to keep rows compact. */
function truncateName(name: string): string {
  if (name.length <= ANCHOR_NAME_MAX) return name;
  return `${name.slice(0, ANCHOR_NAME_MAX - 1)}…`;
}

/** Day-bucket label derived from an anchor's ISO timestamp. The cheapest
 *  grouping the existing `SessionAnchor` shape supports — `timestamp` is
 *  already on the wire (see `web/src/api/types.ts`), and bucketing by
 *  local-day matches how a long-running user scans "where am I in this
 *  session?". `Today` / `Yesterday` get friendly labels; older buckets
 *  fall back to `MMM D`. */
function dayBucketLabel(iso: string, now: Date): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return 'Unknown';
  const startOfDay = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const dayDiff = Math.round((startOfDay(now) - startOfDay(d)) / 86_400_000);
  if (dayDiff === 0) return 'Today';
  if (dayDiff === 1) return 'Yesterday';
  return d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
}

export interface SessionAnchorMinimapProps {
  /** Anchors for this session in append order. The minimap renders one
   *  row per entry; an empty array renders a single muted empty-state
   *  line so the rail does not silently disappear when a new session
   *  has no anchors yet. */
  anchors: SessionAnchor[];
  /** `anchor_id` of the currently-displayed segment, or `null` when no
   *  segment is selected (in which case the most-recent anchor — the
   *  last entry in `anchors` — is treated as "you are here"). The row
   *  matching this id renders with a left-edge accent bar + tinted
   *  background so the user can answer "where am I in this session?"
   *  by glancing. */
  currentAnchorId: number | null;
  /** Invoked with the clicked anchor and its successor (or `null` when
   *  the clicked anchor is the most recent). The parent typically calls
   *  `fetchSessionMessagesBetweenAnchors` and passes the result back
   *  into `TimelineView`. Same shape as the retired
   *  `TimelineChapterStrip` so the data flow in `Chat.tsx` is
   *  unchanged. */
  onSelectAnchor: (from: SessionAnchor, to: SessionAnchor | null) => void;
}

/** One anchor row plus the day-bucket header that precedes it (or
 *  `null` when this row inherits the previous row's bucket). */
interface MinimapRow {
  anchor: SessionAnchor;
  index: number;
  bucket: string | null;
}

/** Group anchors into day-bucket sections in append order. The reducer
 *  walks the array once and emits a header before the first row of each
 *  new bucket so the rendered list stays a single flat sequence — no
 *  nested mapping, no second pass for headers. */
function groupRows(anchors: SessionAnchor[], now: Date): MinimapRow[] {
  let lastBucket: string | null = null;
  return anchors.map((anchor, index) => {
    const bucket = dayBucketLabel(anchor.timestamp, now);
    const header = bucket === lastBucket ? null : bucket;
    lastBucket = bucket;
    return { anchor, index, bucket: header };
  });
}

/**
 * Vertical anchor minimap for the Chat right rail. Replaces the
 * horizontal `TimelineChapterStrip` (deleted in this PR) — a
 * `flex-wrap` row of identical chips did not scale to long sessions
 * and offered no "you are here" answer.
 *
 * Visual contract (issue #2052 Decisions §5–§6):
 *
 * - One row per anchor, append order, day-grouped via `timestamp`.
 * - Current position gets a left-edge accent bar + tinted background.
 * - Hover lifts the row background with an explicit `transition-colors`
 *   (no `transition: all`).
 * - `tabular-nums` on the entry-count badge so digit changes don't
 *   shimmy the column.
 * - Active/press feedback: `scale(0.98)` per the
 *   `make-interfaces-feel-better` checklist.
 * - 40px row height keeps the hit area at the principle-#16 minimum
 *   even though the visible chrome is denser.
 * - Empty state is a single muted line, not nothing — the right rail
 *   placement makes a silent disappearance confusing.
 */
export function SessionAnchorMinimap({
  anchors,
  currentAnchorId,
  onSelectAnchor,
}: SessionAnchorMinimapProps) {
  const handleClick = useCallback(
    (index: number) => {
      const from = anchors[index];
      // Successor lookup defines the half-open `[from, to)` segment.
      // The most-recent anchor has no successor → caller passes `null`,
      // which the API helper translates to "no `to_anchor` query param"
      // → backend reads to EOF. Same contract as the retired strip so
      // `Chat.tsx`'s `handleSelectAnchor` is unchanged.
      const to = index + 1 < anchors.length ? (anchors[index + 1] ?? null) : null;
      if (!from) return;
      onSelectAnchor(from, to);
    },
    [anchors, onSelectAnchor],
  );

  // Resolve "you are here" once per render. When nothing is explicitly
  // selected, the most-recent anchor is the implicit current position —
  // matches how the user reads the conversation (latest first) without
  // needing the parent to seed a click.
  const effectiveCurrentId = useMemo<number | null>(() => {
    if (currentAnchorId !== null) return currentAnchorId;
    const last = anchors[anchors.length - 1];
    return last?.anchor_id ?? null;
  }, [anchors, currentAnchorId]);

  // `now` is captured once per render and threaded into the bucketing
  // reducer so day labels stay consistent across the same paint.
  const rows = useMemo(() => groupRows(anchors, new Date()), [anchors]);

  if (anchors.length === 0) {
    return (
      <div
        data-testid="anchor-minimap-empty"
        className="rounded-md border border-dashed border-border px-2 py-3 text-[11px] text-muted-foreground"
      >
        No anchors yet
      </div>
    );
  }

  return (
    <div
      data-testid="anchor-minimap"
      role="list"
      aria-label="Session anchors"
      // Outer card uses `rounded-lg` (8px). Inner rows use `rounded-md`
      // (6px) — concentric radii per principle #1 (outer = inner +
      // 2px padding gap).
      className="flex flex-col gap-0.5 rounded-lg border border-border bg-card p-1"
    >
      {rows.map(({ anchor, index, bucket }) => {
        const isCurrent = anchor.anchor_id === effectiveCurrentId;
        return (
          <div key={anchor.anchor_id} role="listitem">
            {bucket !== null && (
              <div
                data-testid="anchor-minimap-bucket"
                className="px-2 pb-0.5 pt-1 text-[10px] font-medium uppercase tracking-wide text-muted-foreground"
              >
                {bucket}
              </div>
            )}
            <button
              type="button"
              data-testid="anchor-minimap-row"
              data-anchor-id={anchor.anchor_id}
              data-current={isCurrent ? 'true' : undefined}
              aria-current={isCurrent ? 'true' : undefined}
              title={anchor.name}
              onClick={() => handleClick(index)}
              className={[
                // Hit area: 40px tall row keeps principle #16 satisfied
                // even with the denser visual chrome.
                'group relative flex h-10 w-full items-center gap-2 rounded-md pl-3 pr-2',
                // Explicit transition properties (principle #14 — never
                // `transition: all`). Press feedback at scale-[0.98]
                // (principle #12 / make-interfaces-feel-better §12).
                'transition-[background-color,color,transform] duration-150 active:scale-[0.98]',
                isCurrent
                  ? 'bg-accent text-accent-foreground'
                  : 'text-muted-foreground hover:bg-accent/50 hover:text-foreground',
              ].join(' ')}
            >
              {/*
               * Left-edge accent bar — the "you are here" indicator
               * (Decisions §6 explicitly calls this out, not just bold
               * text). Rendered for every row to keep the row geometry
               * stable; opacity flips so the indented text doesn't jump
               * sideways when the current row changes.
               */}
              <span
                aria-hidden="true"
                data-testid="anchor-minimap-current-bar"
                className={[
                  'absolute left-0 top-1.5 bottom-1.5 w-0.5 rounded-r-sm bg-primary',
                  'transition-opacity duration-150',
                  isCurrent ? 'opacity-100' : 'opacity-0',
                ].join(' ')}
              />
              <Bookmark className="h-3 w-3 shrink-0" aria-hidden="true" />
              <span className="flex-1 truncate text-left font-mono text-[11px]">
                {truncateName(anchor.name)}
              </span>
              <span
                data-testid="anchor-minimap-row-count"
                className="ml-1 rounded bg-muted px-1 text-[10px] tabular-nums text-foreground"
              >
                {anchor.entry_count_in_segment}
              </span>
            </button>
          </div>
        );
      })}
    </div>
  );
}
