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

import { AlertCircle, Bot, CheckCircle2, ChevronDown, Maximize2, Square } from 'lucide-react';
import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react';

import type { LiveRun } from './live-run-store';
import { formatDuration } from './time-format';

import type { TimelineItem } from '@/api/kernel-types';
import { redactObject } from '@/lib/redact';
import { cn } from '@/lib/utils';

interface Props {
  run: LiveRun;
  agentName?: string;
  onOpenTranscript: () => void;
  onStop?: () => void;
}

/** Folded header + collapsible timeline for a single active run. */
export function SingleAgentLiveCard({ run, agentName = 'rara', onOpenTranscript, onStop }: Props) {
  const [expanded, setExpanded] = useState(true);
  const [nowTick, setNowTick] = useState(() => Date.now());
  const scrollerRef = useRef<HTMLDivElement>(null);
  const [stickToBottom, setStickToBottom] = useState(true);
  const [showLatest, setShowLatest] = useState(false);

  // 1 Hz tick while the run is live so the elapsed chip updates.
  useEffect(() => {
    if (run.status !== 'running') return;
    const id = window.setInterval(() => setNowTick(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [run.status]);

  // Auto-scroll when the user is pinned to bottom.
  useEffect(() => {
    if (!expanded) return;
    const el = scrollerRef.current;
    if (!el) return;
    if (stickToBottom) {
      el.scrollTop = el.scrollHeight;
      setShowLatest(false);
    } else {
      setShowLatest(true);
    }
  }, [run.items.length, expanded, stickToBottom]);

  const onScroll = () => {
    const el = scrollerRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
    setStickToBottom(atBottom);
    setShowLatest(!atBottom);
  };

  const elapsed = (run.endedAt ?? nowTick) - run.startedAt;
  const headerLabel = `${agentName} is working`;
  const redactedItems = useMemo(() => run.items.map(redactItem), [run.items]);
  const chips = useMemo(() => buildToolChips(redactedItems), [redactedItems]);

  const onHeaderKey = (ev: KeyboardEvent<HTMLDivElement>) => {
    if (ev.key === 'Enter' || ev.key === ' ') {
      ev.preventDefault();
      setExpanded((v) => !v);
    }
  };

  return (
    <section className="overflow-hidden rounded-lg border border-border/50 bg-card/60 backdrop-blur-sm">
      <div
        role="button"
        tabIndex={0}
        aria-expanded={expanded}
        aria-label={`${headerLabel}; ${run.items.length} events; ${run.toolCalls} tool calls`}
        onClick={() => setExpanded((v) => !v)}
        onKeyDown={onHeaderKey}
        className="flex cursor-pointer items-center gap-2 px-3 py-2 hover:bg-accent/40"
      >
        <Bot className="h-4 w-4 shrink-0 text-muted-foreground" />
        <span
          className="flex h-2 w-2 shrink-0 animate-pulse rounded-full bg-emerald-500"
          aria-hidden
        />
        <span className="min-w-0 flex-1 truncate text-sm font-medium text-foreground">
          {headerLabel}
          {run.currentStage && (
            <span className="ml-2 font-normal text-muted-foreground">
              · {stageLabel(run.currentStage)}
            </span>
          )}
        </span>
        <span className="shrink-0 text-xs tabular-nums text-muted-foreground">
          {formatDuration(elapsed)}
        </span>
        <span className="shrink-0 rounded-full border border-border/60 bg-muted/40 px-2 py-0.5 text-[10px] tabular-nums text-muted-foreground">
          {run.toolCalls} tool{run.toolCalls === 1 ? '' : 's'}
        </span>
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            onOpenTranscript();
          }}
          className="inline-flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition hover:bg-accent hover:text-foreground"
          aria-label="Open full transcript"
          title="Open transcript"
        >
          <Maximize2 className="h-3.5 w-3.5" />
        </button>
        <button
          type="button"
          disabled={!onStop}
          onClick={(e) => {
            e.stopPropagation();
            onStop?.();
          }}
          className="inline-flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition hover:bg-accent hover:text-foreground disabled:cursor-not-allowed disabled:opacity-40"
          aria-label="Stop task"
          title={onStop ? 'Stop' : 'Cancel not yet wired'}
        >
          <Square className="h-3.5 w-3.5" />
        </button>
        <ChevronDown
          className={cn(
            'h-4 w-4 shrink-0 text-muted-foreground transition-transform',
            expanded && 'rotate-180',
          )}
          aria-hidden
        />
      </div>

      {expanded && (
        <div className="relative border-t border-border/50">
          <div
            ref={scrollerRef}
            onScroll={onScroll}
            className="max-h-[213px] overflow-y-auto [-ms-overflow-style:none] [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
          >
            {redactedItems.length === 0 ? (
              <div className="flex items-center gap-2 px-4 py-3 text-xs text-muted-foreground">
                <span
                  className="inline-block h-1.5 w-1.5 shrink-0 animate-pulse rounded-full bg-emerald-500"
                  aria-hidden
                />
                <span className="truncate">{stageLabel(run.currentStage)}</span>
              </div>
            ) : (
              <div className="flex flex-col gap-1 px-3 py-2">
                {chips.map((chip) => (
                  <ToolChip key={`l-${chip.seq}`} chip={chip} />
                ))}
              </div>
            )}
          </div>
          {showLatest && (
            <button
              type="button"
              onClick={() => {
                const el = scrollerRef.current;
                if (el) {
                  el.scrollTop = el.scrollHeight;
                  setStickToBottom(true);
                  setShowLatest(false);
                }
              }}
              className="absolute bottom-2 right-3 rounded-full border border-border/60 bg-background/90 px-2 py-1 text-[10px] text-muted-foreground shadow-sm transition hover:bg-accent"
            >
              Latest
            </button>
          )}
        </div>
      )}
    </section>
  );
}

function redactItem(item: TimelineItem): TimelineItem {
  if (item.kind === 'tool_use' && item.input) {
    return { ...item, input: redactObject(item.input) as Record<string, unknown> };
  }
  return item;
}

interface ToolChipModel {
  /** Source `tool_use` seq — used as React key. */
  seq: number;
  tool: string;
  /** Derived short preview from the (redacted) input. Empty if nothing useful. */
  preview: string;
  status: 'running' | 'completed' | 'errored';
  /** Short error text when `status === 'errored'`. Replaces the preview in the UI. */
  errorText?: string;
}

/**
 * Fold timeline items into one chip per `tool_use`, pairing each use with the
 * nearest following `tool_result` / `error` that shares the same `tool` name.
 *
 * Pairing note: `TimelineItem` deliberately does not expose the backend
 * `tool_call_id` (see `live-run-store.ts` WeakMap comment), so pairing is a
 * best-effort name-order heuristic. If the same tool is invoked twice before
 * the first result lands, chips briefly share state — acceptable for a live
 * indicator.
 *
 * Newest chip first (hermes pattern) so the most recent activity sits at the
 * top of the compact panel.
 */
function buildToolChips(items: readonly TimelineItem[]): ToolChipModel[] {
  const chips: ToolChipModel[] = [];
  for (let i = 0; i < items.length; i++) {
    const use = items[i];
    if (!use || use.kind !== 'tool_use' || !use.tool) continue;
    const toolName = use.tool;
    // Default to running when no pairing follow-up is found; a tool_use
    // without a matching tool_result/error is still in-flight from the UI's
    // perspective even if streaming has technically stopped.
    let status: ToolChipModel['status'] = 'running';
    let errorText: string | undefined;
    for (let j = i + 1; j < items.length; j++) {
      const follow = items[j];
      if (!follow) continue;
      if (follow.kind === 'tool_result' && follow.tool === toolName) {
        status = follow.success === false ? 'errored' : 'completed';
        if (status === 'errored' && follow.output) errorText = clip(follow.output);
        break;
      }
      if (follow.kind === 'error') {
        status = 'errored';
        errorText = clip(follow.content ?? '');
        break;
      }
    }
    const chip: ToolChipModel = {
      seq: use.seq,
      tool: toolName,
      preview: derivePreview(use.input),
      status,
    };
    if (errorText !== undefined) chip.errorText = errorText;
    chips.push(chip);
  }
  return chips.reverse();
}

/** Keys to try in priority order when deriving a short preview from tool input. */
const PREVIEW_KEYS = [
  'query',
  'file_path',
  'path',
  'pattern',
  'description',
  'command',
  'prompt',
  'skill',
] as const;

function derivePreview(input: Record<string, unknown> | undefined): string {
  if (!input) return '';
  for (const key of PREVIEW_KEYS) {
    const v = input[key];
    if (typeof v === 'string' && v.length > 0) {
      const shaped = key === 'file_path' || key === 'path' ? shortenPath(v) : v;
      return clip(shaped);
    }
  }
  for (const v of Object.values(input)) {
    if (typeof v === 'string' && v.length > 0) return clip(v);
  }
  return '';
}

/** Clip long strings to ~110 chars with a trailing ellipsis. */
function clip(s: string): string {
  const limit = 110;
  const flat = s.replace(/\s+/g, ' ').trim();
  return flat.length > limit ? flat.slice(0, limit) + '…' : flat;
}

/** Render long paths as `…/<last-two-segments>` to keep chips one-line. */
function shortenPath(p: string): string {
  const parts = p.split('/').filter(Boolean);
  if (parts.length <= 2) return p;
  return '…/' + parts.slice(-2).join('/');
}

function ToolChip({ chip }: { chip: ToolChipModel }) {
  const body = chip.status === 'errored' && chip.errorText ? chip.errorText : chip.preview;
  return (
    <div className="flex items-center gap-2 rounded-sm bg-muted/40 px-2 py-1 text-[11px]">
      <StatusIcon status={chip.status} />
      <span className="shrink-0 font-mono text-foreground">{chip.tool}</span>
      {body && <span className="min-w-0 flex-1 truncate text-muted-foreground">{body}</span>}
    </div>
  );
}

function StatusIcon({ status }: { status: ToolChipModel['status'] }) {
  if (status === 'running') {
    return (
      <span
        role="status"
        aria-label="running"
        className="inline-block h-3 w-3 shrink-0 animate-spin rounded-full border border-muted-foreground/30 border-t-muted-foreground"
      />
    );
  }
  if (status === 'completed') {
    return <CheckCircle2 aria-label="completed" className="h-3 w-3 shrink-0 text-emerald-500" />;
  }
  return <AlertCircle aria-label="errored" className="h-3 w-3 shrink-0 text-destructive" />;
}

/**
 * Beautify well-known kernel stage strings; raw free-text falls through.
 * Keep the mapping small — kernel stage strings are already intentionally
 * human-readable (see `crates/kernel/src/agent/mod.rs` emit sites); we
 * only translate the two bare identifiers that look like machine tokens
 * in the UI.
 */
function stageLabel(stage: string | null): string {
  if (!stage) return '正在处理…';
  if (stage === 'thinking') return '思考中…';
  if (stage === 'interrupted') return '已中断，准备处理新消息…';
  return stage;
}
