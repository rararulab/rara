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

import { Zap } from 'lucide-react';
import { Fragment, useCallback, useMemo, useRef, useState } from 'react';

import { SessionHeader } from './SessionHeader';
import { TimelineBar } from './TimelineBar';
import { TimelineRow } from './TimelineRow';
import { TurnToolcallCard, isToolcallItem } from './TurnToolcallCard';

import type { TimelineItem } from '@/api/kernel-types';
import { Skeleton } from '@/components/ui/skeleton';
import { useSessionTimeline } from '@/hooks/use-session-timeline';

interface SessionStats {
  agent_id: string;
  manifest_name: string;
  state: string;
  uptime_ms: number;
  llm_calls: number;
  tool_calls: number;
  tokens_consumed: number;
}

export interface SessionDetailProps {
  session: SessionStats;
  /** When false, disables the 5s turns polling (respects Auto-refresh). */
  autoRefresh?: boolean;
}

/**
 * Right panel: session header + TimelineBar + event list.
 *
 * Consumes `useSessionTimeline` and renders the full execution trace.
 * Wrapped by `KernelTop` — selected session is passed from the list.
 *
 * Rendering strategy: non-tool timeline items (`thinking`, `agent`,
 * `plan_card`, ...) map to a `TimelineRow`; consecutive tool-call items
 * from the same turn collapse into a single `TurnToolcallCard` so the
 * compact chip view replaces the verbose per-call rows.
 */
export function SessionDetail({ session, autoRefresh }: SessionDetailProps) {
  const timeline = useSessionTimeline(session.agent_id, session.state, autoRefresh);
  const [selectedIdx, setSelectedIdx] = useState<number | null>(null);
  const rowRefs = useRef<Map<number, HTMLDivElement>>(new Map());

  const handleSegmentClick = useCallback((idx: number) => {
    setSelectedIdx(idx);
    rowRefs.current.get(idx)?.scrollIntoView({ behavior: 'smooth', block: 'center' });
  }, []);

  const groups = useMemo(
    () => groupTimeline(timeline.items, timeline.historicalItems.length),
    [timeline.items, timeline.historicalItems.length],
  );

  return (
    <div className="flex h-full flex-col">
      <SessionHeader
        manifestName={session.manifest_name}
        agentId={session.agent_id}
        state={session.state}
        uptimeMs={session.uptime_ms}
        llmCalls={session.llm_calls}
        toolCalls={session.tool_calls}
        tokensConsumed={session.tokens_consumed}
        isStreaming={timeline.isStreaming}
      />

      {/* TimelineBar */}
      {timeline.items.length > 0 && (
        <div className="border-b px-4 py-2.5">
          <TimelineBar
            items={timeline.items}
            selectedIdx={selectedIdx}
            onSegmentClick={handleSegmentClick}
          />
        </div>
      )}

      {/* Event list */}
      <div className="flex-1 overflow-y-auto min-h-0">
        {timeline.isLoading ? (
          <div className="space-y-2 p-4">
            <Skeleton className="h-8 w-full" />
            <Skeleton className="h-8 w-full" />
            <Skeleton className="h-8 w-full" />
          </div>
        ) : timeline.isError ? (
          <div className="p-6 text-center text-sm italic text-muted-foreground">
            Failed to load execution trace
          </div>
        ) : timeline.items.length === 0 ? (
          <div className="flex flex-1 flex-col items-center justify-center gap-2 p-6 text-center text-muted-foreground">
            <Zap className="h-6 w-6 opacity-20" />
            <p className="text-xs">No events recorded</p>
          </div>
        ) : (
          <div className="divide-y">
            {groups.map((group) => {
              if (group.kind === 'toolcall') {
                return (
                  <Fragment key={group.key}>
                    {group.turnHeader && (
                      <div className="bg-muted/40 px-4 py-1 text-[10px] uppercase tracking-wider text-muted-foreground">
                        Turn #{group.turn + 1}
                      </div>
                    )}
                    <TurnToolcallCard items={group.items} turn={group.turn} isLive={group.isLive} />
                  </Fragment>
                );
              }
              const { item, absoluteIdx, turnHeader } = group;
              return (
                <Fragment key={group.key}>
                  {turnHeader && (
                    <div className="bg-muted/40 px-4 py-1 text-[10px] uppercase tracking-wider text-muted-foreground">
                      Turn #{item.turn + 1}
                    </div>
                  )}
                  <TimelineRow
                    ref={(el) => {
                      if (el) rowRefs.current.set(absoluteIdx, el);
                      else rowRefs.current.delete(absoluteIdx);
                    }}
                    item={item}
                    isSelected={selectedIdx === absoluteIdx}
                    onClick={() =>
                      setSelectedIdx((prev) => (prev === absoluteIdx ? null : absoluteIdx))
                    }
                  />
                </Fragment>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}

/** Rendered slot in the event list — either a single row or a toolcall card. */
type TimelineGroup =
  | {
      kind: 'row';
      key: string;
      item: TimelineItem;
      absoluteIdx: number;
      turnHeader: boolean;
    }
  | {
      kind: 'toolcall';
      key: string;
      turn: number;
      items: TimelineItem[];
      isLive: boolean;
      turnHeader: boolean;
    };

/**
 * Split a flat `TimelineItem[]` into renderable groups. Consecutive
 * tool-call items (see {@link isToolcallItem}) sharing the same `turn`
 * collapse into one `toolcall` group; everything else becomes a `row`
 * group. The first item of a new turn carries a `turnHeader` flag so
 * the caller can render the divider consistently.
 */
function groupTimeline(items: TimelineItem[], historicalCount: number): TimelineGroup[] {
  const groups: TimelineGroup[] = [];
  let prevTurn: number | null = null;
  let i = 0;
  while (i < items.length) {
    const item = items[i];
    if (!item) {
      i++;
      continue;
    }
    const turnHeader = prevTurn !== null && prevTurn !== item.turn;
    if (isToolcallItem(item)) {
      // Greedy fold all consecutive tool-call items of the same turn.
      const bundle: TimelineItem[] = [];
      const firstSeq = item.seq;
      const turn = item.turn;
      let liveInGroup = false;
      while (i < items.length) {
        const cur = items[i];
        if (!cur || cur.turn !== turn || !isToolcallItem(cur)) break;
        if (i >= historicalCount) liveInGroup = true;
        bundle.push(cur);
        i++;
      }
      groups.push({
        kind: 'toolcall',
        key: `tc-${turn}-${firstSeq}`,
        turn,
        items: bundle,
        isLive: liveInGroup,
        turnHeader,
      });
      prevTurn = turn;
      continue;
    }
    const absoluteIdx = i;
    const isLive = absoluteIdx >= historicalCount;
    groups.push({
      kind: 'row',
      key: `${isLive ? 'l' : 'h'}-${item.turn}-${item.seq}-${absoluteIdx}`,
      item,
      absoluteIdx,
      turnHeader,
    });
    prevTurn = item.turn;
    i++;
  }
  return groups;
}
