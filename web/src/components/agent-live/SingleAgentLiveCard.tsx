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

import { Bot, ChevronDown, Maximize2, Square } from 'lucide-react';
import { useEffect, useRef, useState, type KeyboardEvent } from 'react';

import type { LiveRun } from './live-run-store';
import { formatDuration } from './time-format';

import type { TimelineItem } from '@/api/kernel-types';
import { TimelineRow } from '@/components/kernel/TimelineRow';
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
  const redactedItems = run.items.map(redactItem);

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
          <div ref={scrollerRef} onScroll={onScroll} className="max-h-[320px] overflow-y-auto">
            {redactedItems.length === 0 ? (
              <div className="flex items-center gap-2 px-4 py-3 text-xs text-muted-foreground">
                <span
                  className="inline-block h-1.5 w-1.5 shrink-0 animate-pulse rounded-full bg-emerald-500"
                  aria-hidden
                />
                <span className="truncate">{stageLabel(run.currentStage)}</span>
              </div>
            ) : (
              <div className="divide-y divide-border/40">
                {redactedItems.map((item) => (
                  <TimelineRow key={`l-${item.seq}`} item={item} />
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
