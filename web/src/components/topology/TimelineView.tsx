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

import type { TopologyEventEntry } from '@/hooks/use-topology-subscription';

import { TurnCard, buildTurnsFromEvents } from './TurnCard';

export interface TimelineViewProps {
  /** Root session key being observed (used for empty-state copy). */
  rootSessionKey: string;
  /**
   * Every observed event from the topology subscription. The view filters
   * down to root-session events itself; descendant-session events are
   * intentionally not rendered here (task #6 owns the worker inbox).
   */
  events: TopologyEventEntry[];
}

/**
 * Main-timeline view of an agent's stream of consciousness. Renders one
 * `TurnCard` per agent turn observed on the root session, in arrival
 * order. The current in-flight turn (if any) is rendered last with a
 * `thinking…` footer instead of metrics.
 */
export function TimelineView({ rootSessionKey, events }: TimelineViewProps) {
  const turns = useMemo(() => {
    const rootEvents = events
      .filter((e) => e.sessionKey === rootSessionKey)
      .map((e) => ({ seq: e.seq, event: e.event }));
    return buildTurnsFromEvents(rootEvents);
  }, [events, rootSessionKey]);

  if (turns.length === 0) {
    return (
      <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
        Waiting for the next turn on{' '}
        <span className="ml-1 font-mono">{rootSessionKey}</span>…
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
