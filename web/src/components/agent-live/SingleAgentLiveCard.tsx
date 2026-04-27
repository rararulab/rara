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

import { AUTO_DISMISS_MS, type LiveRun } from './live-run-store';
import { formatDuration } from './time-format';
import { ToolChip, buildToolChips } from './tool-chips';

import type { TimelineItem } from '@/api/kernel-types';
import { redactObject } from '@/lib/redact';
import { cn } from '@/lib/utils';

interface Props {
  run: LiveRun;
  agentName?: string;
  onOpenTranscript: () => void;
  onStop?: () => void;
}

// Window of the auto-dismiss interval reserved for the fade-out
// animation. The store retires the run at AUTO_DISMISS_MS; we begin
// fading the card opacity FADE_OUT_MS earlier so the unmount lines up
// with opacity reaching zero.
const FADE_OUT_MS = 300;

/** Folded header + collapsible timeline for a single active run. */
export function SingleAgentLiveCard({ run, agentName = 'rara', onOpenTranscript, onStop }: Props) {
  const [expanded, setExpanded] = useState(true);
  const [nowTick, setNowTick] = useState(() => Date.now());
  const scrollerRef = useRef<HTMLDivElement>(null);
  const [stickToBottom, setStickToBottom] = useState(true);
  const [showLatest, setShowLatest] = useState(false);
  const [fading, setFading] = useState(false);

  // Trigger the fade-out shortly before the store retires the run. Reset
  // when a new run takes the active slot (different runId) or when the
  // run flips back to running (defensive — should not happen today).
  useEffect(() => {
    // Non-terminal states (running/reconnecting) never fade — the run
    // is still live as far as the user is concerned.
    if (run.status === 'running' || run.status === 'reconnecting') {
      setFading(false);
      return;
    }
    const elapsedSinceEnd = run.endedAt ? Date.now() - run.endedAt : 0;
    const fadeAt = Math.max(0, AUTO_DISMISS_MS - FADE_OUT_MS - elapsedSinceEnd);
    const id = window.setTimeout(() => setFading(true), fadeAt);
    return () => {
      window.clearTimeout(id);
      setFading(false);
    };
  }, [run.runId, run.status, run.endedAt]);

  // 1 Hz tick while the run is live so the elapsed chip updates.
  useEffect(() => {
    if (run.status !== 'running' && run.status !== 'reconnecting') return;
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

  // Treat `reconnecting` as a soft-running state for header chrome —
  // the run is alive on the backend; we're just briefly off the wire.
  const isRunning = run.status === 'running' || run.status === 'reconnecting';
  const elapsed = (run.endedAt ?? nowTick) - run.startedAt;
  // A turn that failed before producing any LLM iteration or tool call
  // (e.g. Kimi 403 quota on the very first request) has nothing to show
  // in the duration / tool-count chips, and the generic "encountered an
  // error" copy is not actionable. Suppress the noisy chrome and render
  // a category-specific banner instead (see #1926).
  const failedWithNoWork = run.status === 'failed' && run.toolCalls === 0 && run.items.length === 0;
  const headerLabel =
    run.status === 'running'
      ? `${agentName} is working`
      : run.status === 'reconnecting'
        ? `${agentName} is reconnecting…`
        : run.status === 'failed'
          ? failedTitle(run.errorCategory, agentName)
          : run.status === 'cancelled'
            ? `${agentName} was interrupted`
            : `${agentName} finished`;
  const redactedItems = useMemo(() => run.items.map(redactItem), [run.items]);
  // Newest chip first (hermes pattern) while running; chronological order is
  // nicer once the turn settles so the viewer can read the recap top-down.
  const chips = useMemo(() => {
    const built = buildToolChips(redactedItems);
    return isRunning ? built.slice().reverse() : built;
  }, [redactedItems, isRunning]);

  const onHeaderKey = (ev: KeyboardEvent<HTMLDivElement>) => {
    if (ev.key === 'Enter' || ev.key === ' ') {
      ev.preventDefault();
      setExpanded((v) => !v);
    }
  };

  return (
    <section
      className={cn(
        'overflow-hidden rounded-lg border border-border/50 bg-card/60 backdrop-blur-sm transition-opacity duration-300 ease-out',
        fading && 'opacity-0',
      )}
    >
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
        <RunStatusDot status={run.status} />
        <span className="min-w-0 flex-1 truncate text-sm font-medium text-foreground">
          {headerLabel}
          {isRunning && run.currentStage && (
            <span className="ml-2 font-normal text-muted-foreground">
              · {stageLabel(run.currentStage)}
            </span>
          )}
        </span>
        {!failedWithNoWork && (
          <span className="shrink-0 text-xs tabular-nums text-muted-foreground">
            {formatDuration(elapsed)}
          </span>
        )}
        {!failedWithNoWork && (
          <span className="shrink-0 rounded-full border border-border/60 bg-muted/40 px-2 py-0.5 text-[10px] tabular-nums text-muted-foreground">
            {run.toolCalls} tool{run.toolCalls === 1 ? '' : 's'}
          </span>
        )}
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
        {isRunning && (
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
        )}
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
          {run.status === 'failed' && (
            <FailureBanner
              category={run.errorCategory}
              detail={run.errorDetail ?? run.error}
              upgradeUrl={run.upgradeUrl}
            />
          )}
          <div
            ref={scrollerRef}
            onScroll={onScroll}
            className="max-h-[213px] overflow-y-auto [-ms-overflow-style:none] [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
          >
            {redactedItems.length === 0 ? (
              failedWithNoWork ? null : (
                <div className="flex items-center gap-2 px-4 py-3 text-xs text-muted-foreground">
                  {isRunning && (
                    <span
                      className="inline-block h-1.5 w-1.5 shrink-0 animate-pulse rounded-full bg-emerald-500"
                      aria-hidden
                    />
                  )}
                  <span className="truncate">
                    {isRunning ? stageLabel(run.currentStage) : 'No tool calls in this run'}
                  </span>
                </div>
              )
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

/**
 * Leading indicator dot in the header. Pulses while running; shows a
 * solid success/error/cancelled marker once the run terminates so the
 * card remains visually distinguishable after `done`/`error`.
 */
function RunStatusDot({ status }: { status: LiveRun['status'] }) {
  if (status === 'running') {
    return (
      <span
        className="flex h-2 w-2 shrink-0 animate-pulse rounded-full bg-emerald-500"
        aria-hidden
      />
    );
  }
  if (status === 'reconnecting') {
    // Amber dot — non-terminal but degraded; mirrors the StatusBadge.
    return (
      <span
        className="flex h-2 w-2 shrink-0 animate-pulse rounded-full bg-amber-500"
        aria-label="reconnecting"
      />
    );
  }
  if (status === 'completed') {
    return <CheckCircle2 aria-label="completed" className="h-3 w-3 shrink-0 text-emerald-500" />;
  }
  if (status === 'failed') {
    return <AlertCircle aria-label="failed" className="h-3 w-3 shrink-0 text-destructive" />;
  }
  return (
    <span
      className="flex h-2 w-2 shrink-0 rounded-full bg-muted-foreground/50"
      aria-label="cancelled"
    />
  );
}

/**
 * Pick a header title for the failed state based on the backend-supplied
 * error category. Older error frames without a category fall back to the
 * legacy generic copy so existing screenshots/tests still match.
 */
function failedTitle(category: string | null, agentName: string): string {
  switch (category) {
    case 'quota':
      return 'Kimi 配额已用完';
    case 'network':
      return '网络异常，请稍后重试';
    case 'context_window':
      return '上下文超长';
    case 'tool':
      return '工具调用失败';
    case 'cancelled':
      return '已取消';
    default:
      return `${agentName} encountered an error`;
  }
}

/**
 * Banner rendered above the timeline for a failed run. Shows the
 * category-specific title's CTA (currently only quota carries an upgrade
 * URL) and stows the raw provider message inside a `<details>` so the
 * card stays compact unless the user opts in.
 */
function FailureBanner({
  category,
  detail,
  upgradeUrl,
}: {
  category: string | null;
  detail: string | null;
  upgradeUrl: string | null;
}) {
  return (
    <div className="border-b border-destructive/30 bg-destructive/10 px-4 py-3 text-xs text-destructive">
      {category === 'quota' && upgradeUrl && (
        <div className="mb-2">
          <a
            href={upgradeUrl}
            target="_blank"
            rel="noopener noreferrer"
            className="inline-flex items-center rounded-md border border-destructive/40 bg-background/80 px-2.5 py-1 text-xs font-medium text-destructive transition hover:bg-destructive hover:text-destructive-foreground"
          >
            升级 Kimi 配额
          </a>
        </div>
      )}
      {detail && (
        <details className="text-[11px] text-destructive/90">
          <summary className="cursor-pointer select-none text-destructive hover:underline">
            显示详情
          </summary>
          <pre className="mt-1 whitespace-pre-wrap break-words font-mono text-[11px]">{detail}</pre>
        </details>
      )}
    </div>
  );
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
