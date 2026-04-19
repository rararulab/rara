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

import { Copy, X, CheckCircle2, AlertTriangle, Loader2, Ban } from 'lucide-react';
import { useMemo, useRef, useState } from 'react';

import type { LiveRun, RunStatus } from './live-run-store';
import { formatClock, formatDuration } from './time-format';

import type { TimelineItem } from '@/api/kernel-types';
import { TimelineBar } from '@/components/kernel/TimelineBar';
import { TimelineRow } from '@/components/kernel/TimelineRow';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { redactObject } from '@/lib/redact';

interface Props {
  run: LiveRun | null;
  open: boolean;
  onClose: () => void;
}

/** Full-screen transcript viewer for a single agent run. */
export function AgentTranscriptDialog({ run, open, onClose }: Props) {
  const [selectedIdx, setSelectedIdx] = useState<number | null>(null);
  const rowRefs = useRef(new Map<number, HTMLDivElement>());

  const redactedItems = useMemo(() => (run ? run.items.map(redactItem) : []), [run]);

  if (!run) return null;

  const copyAll = async () => {
    const text = redactedItems
      .map((it) => renderItemAsText(it))
      .filter((t) => t.length > 0)
      .join('\n');
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      /* user gesture required in some browsers — fail silently */
    }
  };

  const onSegmentClick = (idx: number) => {
    setSelectedIdx(idx);
    const el = rowRefs.current.get(idx);
    if (el) el.scrollIntoView({ behavior: 'smooth', block: 'center' });
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(v) => {
        if (!v) onClose();
      }}
    >
      <DialogContent
        className="flex w-[95vw] max-w-4xl flex-col gap-3 overflow-hidden"
        style={{ height: 'calc(100vh - 4rem)' }}
      >
        <DialogHeader className="flex shrink-0 flex-row items-start justify-between gap-3">
          <div className="min-w-0 flex-1">
            <DialogTitle className="flex items-center gap-2">
              <StatusIcon status={run.status} />
              <span>Agent transcript</span>
              <StatusBadge status={run.status} />
            </DialogTitle>
            <DialogDescription className="sr-only">
              Execution transcript with timeline bar and per-event detail.
            </DialogDescription>
            <div className="mt-1 flex flex-wrap gap-x-3 gap-y-1 text-xs text-muted-foreground">
              <span>Duration {formatDuration(durationOf(run))}</span>
              <span>{run.toolCalls} tool calls</span>
              <span>{run.items.length} events</span>
              <span>Started {formatClock(run.startedAt)}</span>
            </div>
          </div>
          <div className="flex shrink-0 items-center gap-1">
            <button
              type="button"
              onClick={copyAll}
              className="inline-flex items-center gap-1 rounded-md border border-border/60 bg-muted/40 px-2 py-1 text-xs text-muted-foreground transition hover:bg-accent"
              aria-label="Copy transcript"
              title="Copy transcript"
            >
              <Copy className="h-3.5 w-3.5" />
              <span>Copy all</span>
            </button>
            <button
              type="button"
              onClick={onClose}
              className="inline-flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition hover:bg-accent hover:text-foreground"
              aria-label="Close transcript"
              title="Close"
            >
              <X className="h-4 w-4" />
            </button>
          </div>
        </DialogHeader>

        <div className="shrink-0">
          <TimelineBar
            items={redactedItems}
            selectedIdx={selectedIdx}
            onSegmentClick={onSegmentClick}
          />
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto rounded-md border border-border/50 bg-background/30">
          {redactedItems.length === 0 ? (
            <div className="p-6 text-center text-sm text-muted-foreground">
              No events recorded for this run.
            </div>
          ) : (
            <div className="divide-y divide-border/40">
              {redactedItems.map((item, idx) => (
                <TimelineRow
                  key={`l-${item.seq}`}
                  item={item}
                  isSelected={selectedIdx === idx}
                  onClick={() => setSelectedIdx(idx)}
                  ref={(el: HTMLDivElement | null) => {
                    if (el) rowRefs.current.set(idx, el);
                    else rowRefs.current.delete(idx);
                  }}
                />
              ))}
            </div>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}

function StatusBadge({ status }: { status: RunStatus }) {
  const { label, cls } = statusChrome(status);
  return (
    <span className={`rounded-full border px-2 py-0.5 text-[10px] font-medium ${cls}`}>
      {label}
    </span>
  );
}

function StatusIcon({ status }: { status: RunStatus }) {
  switch (status) {
    case 'running':
      return <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />;
    case 'completed':
      return <CheckCircle2 className="h-4 w-4 text-emerald-500" />;
    case 'failed':
      return <AlertTriangle className="h-4 w-4 text-destructive" />;
    case 'cancelled':
      return <Ban className="h-4 w-4 text-muted-foreground" />;
  }
}

function statusChrome(status: RunStatus): { label: string; cls: string } {
  switch (status) {
    case 'running':
      return {
        label: 'Running',
        cls: 'border-border/60 bg-muted/50 text-muted-foreground',
      };
    case 'completed':
      return {
        label: 'Completed',
        cls: 'border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300',
      };
    case 'failed':
      return {
        label: 'Failed',
        cls: 'border-destructive/40 bg-destructive/10 text-destructive',
      };
    case 'cancelled':
      return {
        label: 'Cancelled',
        cls: 'border-border/60 bg-muted/40 text-muted-foreground',
      };
  }
}

/** Produce a plain-text line for clipboard export. */
function renderItemAsText(item: TimelineItem): string {
  switch (item.kind) {
    case 'thinking':
      return `[thinking] ${item.content ?? ''}`;
    case 'tool_use':
      return `[tool:${item.tool ?? '?'}] ${JSON.stringify(item.input ?? {})}`;
    case 'tool_result':
      return `[result:${item.tool ?? '?'}] ${item.output ?? ''}`;
    case 'agent':
      return `[text] ${item.content ?? ''}`;
    case 'error':
      return `[error] ${item.content ?? ''}`;
    default:
      return '';
  }
}

function durationOf(run: LiveRun): number {
  const end = run.endedAt ?? Date.now();
  return Math.max(0, end - run.startedAt);
}

/** Redact secret-like fields in input before the dialog renders them. */
function redactItem(item: TimelineItem): TimelineItem {
  if (item.kind === 'tool_use' && item.input) {
    return { ...item, input: redactObject(item.input) as Record<string, unknown> };
  }
  return item;
}
