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

import { useEffect, useMemo, useRef } from 'react';

import { PromptEditor } from './PromptEditor';
import { TurnCard, buildTurnsFromEvents } from './TurnCard';

import type { TopologyEventEntry } from '@/hooks/use-topology-subscription';

export interface TimelineViewProps {
  /**
   * Session key whose events should be rendered. Defaults to the root
   * when omitted; task #6's worker inbox passes a child session key to
   * focus the timeline on that worker. The view always filters down to a
   * single session — interleaving multiple sessions in one column would
   * break per-turn boundaries (a child's `done` would split the parent's
   * turn and vice versa).
   */
  viewSessionKey: string;
  /**
   * Every observed event from the topology subscription. The view
   * filters down to `viewSessionKey` itself; sibling-session events are
   * rendered in the worker inbox (task #6) and the fork topology view
   * (task #7).
   */
  events: TopologyEventEntry[];
  /**
   * Session key the prompt editor sends into. Usually equal to
   * `viewSessionKey` but kept as a separate prop so callers can leave
   * the editor disabled (`null`) when the user is browsing without an
   * active conversation — e.g. inspecting a finished worker.
   */
  promptSessionKey?: string | null;
}

/**
 * Main-timeline view of an agent's stream of consciousness. Renders one
 * `TurnCard` per agent turn observed on `viewSessionKey`, in arrival
 * order. The current in-flight turn (if any) is rendered last with a
 * `thinking…` footer instead of metrics.
 *
 * Below the turn list sits a craft-style `PromptEditor` pinned to the
 * bottom of the column. The editor is the single inbound surface for
 * this session — sending or aborting messages — so the topology page
 * stops being purely observational. New turns flow back in via the
 * shared topology WS subscription, which means there is no client-side
 * optimistic message; the user's prompt appears as soon as the kernel
 * echoes it.
 */
export function TimelineView({ viewSessionKey, events, promptSessionKey }: TimelineViewProps) {
  const turns = useMemo(() => {
    const sessionEvents = events
      .filter((e) => e.sessionKey === viewSessionKey)
      .map((e) => ({ seq: e.seq, event: e.event }));
    return buildTurnsFromEvents(sessionEvents);
  }, [events, viewSessionKey]);

  // Keep the timeline scrolled to the latest turn so the user follows
  // the live stream without having to scroll. Anchored on turn count +
  // last turn id so a new turn (or new chunks accumulating into the
  // active turn) auto-scrolls; idle re-renders don't force scroll.
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const lastTurnId = turns.at(-1)?.id;
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
  }, [turns.length, lastTurnId]);

  return (
    <div className="flex flex-1 min-h-0 flex-col">
      <div ref={scrollRef} className="flex-1 min-h-0 space-y-3 overflow-y-auto pr-1">
        {turns.length === 0 ? (
          <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
            Waiting for the next turn on <span className="ml-1 font-mono">{viewSessionKey}</span>…
          </div>
        ) : (
          turns.map((turn) => <TurnCard key={turn.id} turn={turn} />)
        )}
      </div>
      <PromptEditor sessionKey={promptSessionKey ?? viewSessionKey} />
    </div>
  );
}
