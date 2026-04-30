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

import { WorkerCard, type WorkerInfo, type WorkerStatus } from './WorkerCard';

import type { TopologyEventEntry } from '@/hooks/use-topology-subscription';

export interface WorkerInboxProps {
  /** Root session key — workers are children of this root (directly or transitively). */
  rootSessionKey: string;
  /** Every observed event from the topology subscription. */
  events: TopologyEventEntry[];
  /**
   * Currently focused session in the main timeline. `null` means the
   * root view is active; any non-null value highlights the matching
   * worker card.
   */
  activeChildSession: string | null;
  /** Click handler — switches the main timeline to a child's events. */
  onSelectChild: (childSession: string) => void;
}

/**
 * Right-rail worker inbox. Derives one card per spawned subagent from
 * the topology event buffer, with status, manifest name, last activity
 * seq, and event count. Completed / failed workers are kept visible —
 * the surface is an observation deck, not a job queue.
 */
export function WorkerInbox({
  rootSessionKey,
  events,
  activeChildSession,
  onSelectChild,
}: WorkerInboxProps) {
  const workers = useMemo(() => deriveWorkers(rootSessionKey, events), [rootSessionKey, events]);

  if (workers.length === 0) {
    return (
      <div className="rounded border border-dashed border-border px-2 py-3 text-[11px] text-muted-foreground">
        No workers spawned yet.
      </div>
    );
  }

  return (
    <div className="space-y-1.5">
      {workers.map((worker) => (
        <WorkerCard
          key={worker.childSession}
          worker={worker}
          active={activeChildSession === worker.childSession}
          onSelect={onSelectChild}
        />
      ))}
    </div>
  );
}

/**
 * Fold the event stream into one {@link WorkerInfo} per spawned child.
 * Pure — re-runs cheaply on every event push via `useMemo`.
 *
 * The root session itself is excluded — the back-to-root affordance
 * lives in the timeline header, not in the inbox.
 */
export function deriveWorkers(rootSessionKey: string, events: TopologyEventEntry[]): WorkerInfo[] {
  const byChild = new Map<string, WorkerInfo>();

  for (const entry of events) {
    const frame = entry.event;
    if (frame.type === 'subagent_spawned') {
      // Spawn marker arrives on the parent session. Use it to seed the
      // worker entry with manifest name + parent linkage.
      const existing = byChild.get(frame.child_session);
      if (existing) {
        existing.manifestName = frame.manifest_name;
        existing.parentSession = frame.parent_session;
      } else {
        byChild.set(frame.child_session, {
          childSession: frame.child_session,
          parentSession: frame.parent_session,
          manifestName: frame.manifest_name,
          status: 'spawned',
          lastActivitySeq: 0,
          eventCount: 0,
        });
      }
      continue;
    }

    if (frame.type === 'subagent_done') {
      // Done marker also arrives on the parent. Promote the worker
      // entry to its terminal status.
      const worker = ensureWorker(byChild, frame.child_session, frame.parent_session);
      worker.status = frame.success ? 'completed' : 'failed';
      continue;
    }

    // Any other frame received with `sessionKey != root` belongs to a
    // worker — bump its activity counters and lift `spawned` to
    // `running`. We don't try to reconstruct manifest_name from these;
    // it always arrives via the `subagent_spawned` frame.
    if (entry.sessionKey === rootSessionKey) continue;
    const worker = ensureWorker(byChild, entry.sessionKey, null);
    worker.eventCount += 1;
    worker.lastActivitySeq = entry.seq;
    if (worker.status === 'spawned') worker.status = 'running';
  }

  return [...byChild.values()];
}

/**
 * Look up a worker entry, creating a placeholder if events arrive for a
 * child before the corresponding `subagent_spawned` frame (the topology
 * `hello` snapshot includes pre-existing descendants but not their
 * manifest names — those come over the wire later).
 */
function ensureWorker(
  byChild: Map<string, WorkerInfo>,
  childSession: string,
  parentSession: string | null,
): WorkerInfo {
  const existing = byChild.get(childSession);
  if (existing) return existing;
  const placeholder: WorkerInfo = {
    childSession,
    parentSession: parentSession ?? '',
    manifestName: '(unknown)',
    status: 'running' satisfies WorkerStatus,
    lastActivitySeq: 0,
    eventCount: 0,
  };
  byChild.set(childSession, placeholder);
  return placeholder;
}
