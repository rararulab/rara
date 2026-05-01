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

import { cn } from '@/lib/utils';

/**
 * Worker lifecycle status surfaced as a badge on each card.
 *
 * - `spawned`   — `subagent_spawned` seen, no events from the child yet
 * - `running`   — at least one child-session event observed
 * - `completed` — `subagent_done` with `success: true`
 * - `failed`    — `subagent_done` with `success: false`
 */
export type WorkerStatus = 'spawned' | 'running' | 'completed' | 'failed';

/** One row in the worker inbox — see {@link WorkerInbox}. */
export interface WorkerInfo {
  childSession: string;
  parentSession: string;
  manifestName: string;
  status: WorkerStatus;
  /** Per-connection seq of the most recent event seen on this child. */
  lastActivitySeq: number;
  /** Number of child-session events observed (excludes spawn / done frames). */
  eventCount: number;
}

export interface WorkerCardProps {
  worker: WorkerInfo;
  active: boolean;
  onSelect: (childSession: string) => void;
}

/**
 * Compact card for one spawned subagent. Click to switch the main
 * timeline view to that child's events; click the back button on the
 * timeline header to return to the root view.
 */
export function WorkerCard({ worker, active, onSelect }: WorkerCardProps) {
  return (
    <button
      type="button"
      onClick={() => onSelect(worker.childSession)}
      title={worker.childSession}
      className={cn(
        'flex w-full flex-col gap-1 rounded border px-2 py-1.5 text-left text-[11px] transition-colors',
        active
          ? 'border-accent bg-accent/10 text-foreground'
          : 'border-border bg-card text-foreground hover:bg-muted/40',
      )}
    >
      <div className="flex items-center justify-between gap-2">
        <span className="truncate font-medium">{worker.manifestName}</span>
        <StatusBadge status={worker.status} />
      </div>
      <span className="truncate font-mono text-[10px] text-muted-foreground">
        {shortenSessionKey(worker.childSession)}
      </span>
      <div className="flex items-center justify-between gap-2 text-[10px] text-muted-foreground">
        <span>{worker.eventCount} events</span>
        <span>
          {worker.lastActivitySeq > 0 ? `last @ #${worker.lastActivitySeq}` : 'no events yet'}
        </span>
      </div>
    </button>
  );
}

function StatusBadge({ status }: { status: WorkerStatus }) {
  // Map status → background token. Foreground stays white for the colored
  // states so the pill reads as a solid chip even in dark mode.
  const styles: Record<WorkerStatus, string> = {
    spawned: 'bg-muted text-muted-foreground',
    running: 'bg-info text-white',
    completed: 'bg-success text-white',
    failed: 'bg-destructive text-white',
  };
  return (
    <span
      className={cn(
        'shrink-0 rounded px-1.5 py-px text-[9px] font-medium uppercase tracking-wide',
        styles[status],
      )}
    >
      {status}
    </span>
  );
}

/**
 * Shorten a session key like `mita::01HX...` for the card body. The full
 * key is still in the `title` tooltip on the parent button.
 */
function shortenSessionKey(key: string): string {
  if (key.length <= 24) return key;
  return `${key.slice(0, 12)}…${key.slice(-8)}`;
}
