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
}

/**
 * Main-timeline view of an agent's stream of consciousness. Renders one
 * `TurnCard` per agent turn observed on `viewSessionKey`, in arrival
 * order. The current in-flight turn (if any) is rendered last with a
 * `thinking…` footer instead of metrics.
 */
export function TimelineView({ viewSessionKey, events }: TimelineViewProps) {
  const turns = useMemo(() => {
    const sessionEvents = events
      .filter((e) => e.sessionKey === viewSessionKey)
      .map((e) => ({ seq: e.seq, event: e.event }));
    return buildTurnsFromEvents(sessionEvents);
  }, [events, viewSessionKey]);

  if (turns.length === 0) {
    return (
      <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
        Waiting for the next turn on <span className="ml-1 font-mono">{viewSessionKey}</span>…
      </div>
    );
  }

  return (
    <div className="space-y-3">
      {turns.map((turn) => (
        <TurnCard key={turn.id} turn={turn} />
      ))}
    </div>
  );
}
