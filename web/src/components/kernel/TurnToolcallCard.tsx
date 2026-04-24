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

import { ChevronDown, Wrench } from 'lucide-react';
import { useState } from 'react';

import type { TimelineItem } from '@/api/kernel-types';
import { ToolChip, buildToolChips } from '@/components/agent-live/tool-chips';
import { cn } from '@/lib/utils';

export interface TurnToolcallCardProps {
  /** Tool-related items (`tool_use` / `tool_result` / bare `error`) for one turn. */
  items: TimelineItem[];
  /** 0-based turn index — used only for the React key by the parent. */
  turn: number;
  /** True when any `tool_use` in `items` is still streaming. */
  isLive?: boolean;
}

/**
 * Collapsed per-turn summary of every tool call made within a single
 * agent turn. Replaces the previous inline `tool_use` / `tool_result`
 * rows in `SessionDetail` — the same information in a denser form, with
 * click-to-expand JSON payloads.
 */
export function TurnToolcallCard({ items, isLive = false }: TurnToolcallCardProps) {
  const [expanded, setExpanded] = useState(true);
  const chips = buildToolChips(items);
  if (chips.length === 0) return null;

  const completed = chips.filter((c) => c.status === 'completed').length;
  const errored = chips.filter((c) => c.status === 'errored').length;
  const running = chips.filter((c) => c.status === 'running').length;

  return (
    <section className="px-4 py-2">
      <button
        type="button"
        onClick={() => setExpanded((v) => !v)}
        aria-expanded={expanded}
        className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left transition hover:bg-accent/40"
      >
        <Wrench className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <span className="text-xs font-medium text-foreground">
          {chips.length} tool call{chips.length === 1 ? '' : 's'}
        </span>
        <span className="flex-1 truncate text-[11px] text-muted-foreground">
          {running > 0 && <span className="mr-2">{running} running</span>}
          {completed > 0 && <span className="mr-2 text-emerald-600">{completed} ok</span>}
          {errored > 0 && <span className="mr-2 text-destructive">{errored} errored</span>}
        </span>
        {isLive && (
          <span
            className="h-1.5 w-1.5 shrink-0 animate-pulse rounded-full bg-emerald-500"
            aria-hidden
          />
        )}
        <ChevronDown
          className={cn(
            'h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform',
            expanded && 'rotate-180',
          )}
          aria-hidden
        />
      </button>

      {expanded && (
        <div className="mt-1.5 flex flex-col gap-1 pl-6">
          {chips.map((chip) => (
            <ToolChip key={`c-${chip.seq}`} chip={chip} expandable />
          ))}
        </div>
      )}
    </section>
  );
}

/** Whether a timeline item belongs to the per-turn tool-call card. */
export function isToolcallItem(item: TimelineItem): boolean {
  if (item.kind === 'tool_use' || item.kind === 'tool_result') return true;
  // Bare `error` rows that were emitted from a tool-call end carry a `tool`
  // name — fold them into the card so the chip shows the error text.
  // Turn-level errors (no `tool`) stay inline as a regular error row.
  if (item.kind === 'error' && item.tool) return true;
  return false;
}
