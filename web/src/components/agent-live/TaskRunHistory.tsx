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

import {
  AlertTriangle,
  Ban,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Maximize2,
} from 'lucide-react';
import { useState } from 'react';

import type { LiveRun, RunStatus } from './live-run-store';
import { formatClock, formatDuration } from './time-format';

import type { TimelineItem } from '@/api/kernel-types';
import { TimelineRow } from '@/components/kernel/TimelineRow';
import { redactObject } from '@/lib/redact';
import { cn } from '@/lib/utils';

interface Props {
  runs: LiveRun[];
  onOpenTranscript: (run: LiveRun) => void;
}

/** Collapsible history section listing completed/failed/cancelled runs. */
export function TaskRunHistory({ runs, onOpenTranscript }: Props) {
  const [open, setOpen] = useState(false);

  if (runs.length === 0) return null;

  return (
    <section className="rounded-lg border border-border/50 bg-card/40 backdrop-blur">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center gap-2 px-3 py-2 text-left text-xs text-muted-foreground transition hover:bg-accent/40"
        aria-expanded={open}
      >
        {open ? <ChevronDown className="h-3.5 w-3.5" /> : <ChevronRight className="h-3.5 w-3.5" />}
        <span>Execution history</span>
        <span className="text-[10px] tabular-nums">({runs.length})</span>
      </button>
      {open && (
        <ul className="divide-y divide-border/40 border-t border-border/50">
          {runs.map((run) => (
            <HistoryItem key={run.runId} run={run} onOpenTranscript={onOpenTranscript} />
          ))}
        </ul>
      )}
    </section>
  );
}

function HistoryItem({
  run,
  onOpenTranscript,
}: {
  run: LiveRun;
  onOpenTranscript: (run: LiveRun) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  // Historical runs always carry `endedAt` (finalize() stamps it when the
  // run leaves the active slot). Falling back to 0 keeps this pure under
  // the React 19 hook purity lint without ever firing in practice.
  const duration = (run.endedAt ?? run.startedAt) - run.startedAt;
  const redactedItems = run.items.map(redactItem);

  return (
    <li>
      <div className="flex items-center gap-2 px-3 py-1.5 text-xs">
        <StatusIcon status={run.status} />
        <span className="tabular-nums text-muted-foreground">{formatClock(run.startedAt)}</span>
        <span className="tabular-nums text-muted-foreground/70">{formatDuration(duration)}</span>
        <span className={cn('text-xs', statusTextClass(run.status))}>
          {statusLabel(run.status)}
        </span>
        <span className="ml-2 text-[11px] text-muted-foreground/70">
          {run.toolCalls} tool{run.toolCalls === 1 ? '' : 's'}
        </span>
        <div className="ml-auto flex items-center gap-1">
          <button
            type="button"
            onClick={() => setExpanded((v) => !v)}
            className="inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[11px] text-muted-foreground transition hover:bg-accent"
            aria-expanded={expanded}
          >
            {expanded ? 'Collapse' : 'Expand'}
          </button>
          <button
            type="button"
            onClick={() => onOpenTranscript(run)}
            className="inline-flex h-6 w-6 items-center justify-center rounded text-muted-foreground transition hover:bg-accent hover:text-foreground"
            aria-label="Open full transcript"
            title="Open transcript"
          >
            <Maximize2 className="h-3 w-3" />
          </button>
        </div>
      </div>
      {expanded && (
        <div className="border-t border-border/40 bg-background/30">
          {redactedItems.length === 0 ? (
            <div className="px-4 py-3 text-center text-[11px] text-muted-foreground">
              No events recorded for this run.
            </div>
          ) : (
            <div className="divide-y divide-border/40">
              {redactedItems.map((item) => (
                <TimelineRow key={`l-${item.seq}`} item={item} />
              ))}
            </div>
          )}
        </div>
      )}
    </li>
  );
}

function StatusIcon({ status }: { status: RunStatus }) {
  switch (status) {
    case 'completed':
      return <CheckCircle2 className="h-3.5 w-3.5 text-emerald-500" />;
    case 'failed':
      return <AlertTriangle className="h-3.5 w-3.5 text-destructive" />;
    case 'cancelled':
      return <Ban className="h-3.5 w-3.5 text-muted-foreground" />;
    case 'running':
      // Unreachable — the history section never renders running runs.
      return <CheckCircle2 className="h-3.5 w-3.5 text-muted-foreground" />;
  }
}

function statusLabel(status: RunStatus): string {
  switch (status) {
    case 'completed':
      return 'Completed';
    case 'failed':
      return 'Failed';
    case 'cancelled':
      return 'Cancelled';
    case 'running':
      return 'Running';
  }
}

function statusTextClass(status: RunStatus): string {
  switch (status) {
    case 'completed':
      return 'text-emerald-700 dark:text-emerald-300';
    case 'failed':
      return 'text-destructive';
    default:
      return 'text-muted-foreground';
  }
}

function redactItem(item: TimelineItem): TimelineItem {
  if (item.kind === 'tool_use' && item.input) {
    return { ...item, input: redactObject(item.input) as Record<string, unknown> };
  }
  return item;
}
