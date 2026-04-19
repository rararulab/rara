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

import { Fragment, useCallback, useRef, useState } from 'react';
import { Zap } from 'lucide-react';
import { useSessionTimeline } from '@/hooks/use-session-timeline';
import { Skeleton } from '@/components/ui/skeleton';
import { TimelineBar } from './TimelineBar';
import { TimelineRow } from './TimelineRow';
import { SessionHeader } from './SessionHeader';

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
 */
export function SessionDetail({ session, autoRefresh }: SessionDetailProps) {
  const timeline = useSessionTimeline(session.agent_id, session.state, autoRefresh);
  const [selectedIdx, setSelectedIdx] = useState<number | null>(null);
  const rowRefs = useRef<Map<number, HTMLDivElement>>(new Map());

  const handleSegmentClick = useCallback((idx: number) => {
    setSelectedIdx(idx);
    rowRefs.current.get(idx)?.scrollIntoView({ behavior: 'smooth', block: 'center' });
  }, []);

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
            {timeline.items.map((item, idx) => {
              const prev = timeline.items[idx - 1];
              const turnChanged = idx > 0 && (!prev || prev.turn !== item.turn);
              const isLive = idx >= timeline.historicalItems.length;
              const rowKey = `${isLive ? 'l' : 'h'}-${item.turn}-${item.seq}-${idx}`;
              return (
                <Fragment key={rowKey}>
                  {turnChanged && (
                    <div className="bg-muted/40 px-4 py-1 text-[10px] uppercase tracking-wider text-muted-foreground">
                      Turn #{item.turn + 1}
                    </div>
                  )}
                  <TimelineRow
                    ref={(el) => {
                      if (el) rowRefs.current.set(idx, el);
                      else rowRefs.current.delete(idx);
                    }}
                    item={item}
                    isSelected={selectedIdx === idx}
                    onClick={() => setSelectedIdx((prev) => (prev === idx ? null : idx))}
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
