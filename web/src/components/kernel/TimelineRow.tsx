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

import { forwardRef, useEffect, useState } from "react";
import { AlertCircle, Brain, ChevronRight, Loader2 } from "lucide-react";
import { cn } from "@/lib/utils";
import type {
  BackgroundTaskInfo,
  PlanState,
  PlanStepUiStatus,
  TimelineItem,
  TurnUsage,
} from "@/api/kernel-types";
import { KIND_PALETTE, eventLabel, eventSummary } from "./timeline-colors";

const DETAIL_MAX_CHARS = 4000;

export interface TimelineRowProps {
  item: TimelineItem;
  /** When true, apply selected background (used by TimelineBar scroll-to). */
  isSelected?: boolean;
  onClick?: () => void;
}

/**
 * One event row in the session timeline.
 *
 * Layout: `[ColoredBadge 60px]` `[summary truncate]` `[#seq mono 10px]`.
 * When the item has expandable detail (tool JSON / long text / result
 * preview), the summary becomes a button that toggles a muted detail
 * panel underneath.
 */
export const TimelineRow = forwardRef<HTMLDivElement, TimelineRowProps>(
  function TimelineRow({ item, isSelected, onClick }, ref) {
    const [expanded, setExpanded] = useState(false);

    // Plan cards render a self-contained widget (goal header, step list,
    // footer) rather than the standard badge+summary row layout.
    if (item.kind === "plan_card" && item.plan) {
      return (
        <div
          ref={ref}
          className={cn(
            "group transition-colors",
            isSelected && "bg-accent/50",
          )}
          onClick={onClick}
        >
          <PlanCardBody item={item} plan={item.plan} />
        </div>
      );
    }

    // Token footer is a terminal, decoration-only row — no badge, no
    // expand affordance, just the settled per-turn totals.
    if (item.kind === "token_footer" && item.usage) {
      return (
        <div ref={ref} className="px-4 py-1.5">
          <TokenFooterBody usage={item.usage} />
        </div>
      );
    }

    // Background-task chips render as a horizontal pill row with live
    // elapsed timers; no badge / expand affordance.
    if (item.kind === "background_tasks" && item.bgTasks) {
      return (
        <div ref={ref} className="px-4 py-2">
          <BackgroundTasksBody tasks={item.bgTasks} />
        </div>
      );
    }

    const palette = KIND_PALETTE[item.kind];
    const label = eventLabel(item.kind, item.tool);
    const summary = eventSummary(item);
    const hasDetail = rowHasDetail(item);

    return (
      <div
        ref={ref}
        className={cn(
          "group transition-colors",
          isSelected && "bg-accent/50",
        )}
        onClick={onClick}
      >
        <div className="flex items-start gap-2 px-4 py-2">
          <span
            className={cn(
              "mt-0.5 inline-flex min-w-[60px] shrink-0 items-center justify-center rounded px-1.5 py-0.5 text-[11px] font-medium",
              palette.label,
            )}
          >
            {item.kind === "thinking" && (
              <Brain className="mr-1 h-3 w-3 shrink-0" />
            )}
            {item.kind === "in_progress" && (
              <Loader2 className="mr-1 h-3 w-3 shrink-0 animate-spin" />
            )}
            {item.kind === "error" && (
              <AlertCircle className="mr-1 h-3 w-3 shrink-0" />
            )}
            <span className="truncate">{label}</span>
          </span>

          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              if (hasDetail) setExpanded((v) => !v);
            }}
            disabled={!hasDetail}
            className={cn(
              "min-w-0 flex-1 py-0.5 text-left text-xs transition-colors",
              hasDetail
                ? "cursor-pointer hover:text-foreground"
                : "cursor-default",
              item.kind === "error"
                ? "text-destructive"
                : "text-muted-foreground",
            )}
          >
            <div className="flex items-start gap-1.5">
              {hasDetail && (
                <ChevronRight
                  className={cn(
                    "mt-0.5 h-3 w-3 shrink-0 text-muted-foreground/50 transition-transform",
                    expanded && "rotate-90",
                  )}
                />
              )}
              <span className="truncate">
                {summary || <span className="italic opacity-60">(empty)</span>}
                {item.streaming && (
                  <span className="ml-1 inline-block h-3 w-0.5 translate-y-0.5 animate-pulse bg-current align-middle" />
                )}
              </span>
            </div>
          </button>

          <span className="mt-1 shrink-0 text-[10px] tabular-nums text-muted-foreground/50">
            #{item.seq}
          </span>
        </div>

        {hasDetail && (
          <div
            className={cn(
              "grid transition-[grid-template-rows] duration-200 ease-out",
              expanded ? "grid-rows-[1fr]" : "grid-rows-[0fr]",
            )}
          >
            <div className="overflow-hidden">
              <div className="px-4 pb-3">
                <div className="ml-[72px] rounded border bg-muted/40">
                  <RowDetail item={item} />
                </div>
              </div>
            </div>
          </div>
        )}
      </div>
    );
  },
);

/** Whether the item carries content worth a collapsible detail panel. */
function rowHasDetail(item: TimelineItem): boolean {
  switch (item.kind) {
    case "tool_use":
      return !!item.input && Object.keys(item.input).length > 0;
    case "tool_result":
      return !!item.output && item.output.length > 0;
    case "thinking":
    case "error":
      return !!item.content && item.content.length > 0;
    case "agent":
      return !!item.content && item.content.split("\n").length > 1;
    case "in_progress":
      // Placeholder is purely decorative — never expandable.
      return false;
    case "plan_card":
    case "token_footer":
    case "background_tasks":
      // Self-contained widgets — early-returned before this helper runs.
      return false;
  }
}

function RowDetail({ item }: { item: TimelineItem }) {
  switch (item.kind) {
    case "tool_use":
      return (
        <pre className="max-h-60 overflow-auto whitespace-pre-wrap break-all p-3 text-[11px] text-muted-foreground">
          {item.input ? JSON.stringify(item.input, null, 2) : ""}
        </pre>
      );
    case "tool_result":
      return (
        <pre className="max-h-60 overflow-auto whitespace-pre-wrap break-all p-3 text-[11px] text-muted-foreground">
          {truncate(item.output ?? "")}
        </pre>
      );
    case "thinking":
    case "agent":
      return (
        <pre className="max-h-60 overflow-auto whitespace-pre-wrap break-words p-3 text-[11px] text-muted-foreground">
          {item.content ?? ""}
        </pre>
      );
    case "error":
      return (
        <pre className="max-h-60 overflow-auto whitespace-pre-wrap break-words p-3 text-[11px] text-destructive">
          {item.content ?? ""}
        </pre>
      );
    case "in_progress":
      // Never rendered — `rowHasDetail` returns false for this kind.
      return null;
    case "plan_card":
    case "token_footer":
    case "background_tasks":
      // Self-contained widgets — never render via the shared detail pane.
      return null;
  }
}

const STEP_STATUS_ICON: Record<PlanStepUiStatus, string> = {
  pending: "\u{26AA}",
  running: "\u{1F504}",
  done: "\u{2705}",
  failed: "\u{274C}",
  needs_replan: "\u{1F501}",
};

/**
 * Self-contained plan widget: goal header, step list with status icons,
 * nested active tool calls, optional replan/summary footer.
 */
function PlanCardBody({
  item,
  plan,
}: {
  item: TimelineItem;
  plan: PlanState;
}) {
  const palette = KIND_PALETTE.plan_card;
  const completed = plan.steps.filter((s) => s.status === "done").length;

  return (
    <div className="px-4 py-2">
      <div className="flex items-start gap-2">
        <span
          className={cn(
            "mt-0.5 inline-flex min-w-[60px] shrink-0 items-center justify-center rounded px-1.5 py-0.5 text-[11px] font-medium",
            palette.label,
          )}
        >
          <span className="truncate">📋 {palette.text}</span>
        </span>

        <div className="min-w-0 flex-1">
          <div className="flex items-baseline gap-2">
            <span className="truncate text-xs font-medium text-foreground">
              {plan.goal}
            </span>
            <span className="shrink-0 text-[10px] tabular-nums text-muted-foreground/70">
              {completed}/{plan.totalSteps}
            </span>
            {item.streaming && !plan.completed && (
              <span className="inline-block h-3 w-0.5 translate-y-0.5 animate-pulse bg-current align-middle" />
            )}
          </div>

          <ul className="mt-1.5 space-y-0.5">
            {plan.steps.map((step) => (
              <li
                key={step.index}
                className="flex items-start gap-1.5 text-[11px] leading-relaxed"
              >
                <span className="shrink-0" aria-hidden>
                  {STEP_STATUS_ICON[step.status]}
                </span>
                <span
                  className={cn(
                    "min-w-0 flex-1",
                    step.status === "done" &&
                      "text-muted-foreground line-through decoration-muted-foreground/40",
                    step.status === "pending" && "text-muted-foreground/70",
                    (step.status === "failed" ||
                      step.status === "needs_replan") &&
                      "text-destructive",
                  )}
                >
                  <span className="mr-1 tabular-nums text-muted-foreground/60">
                    {step.index}.
                  </span>
                  {step.task || (
                    <span className="italic opacity-60">(pending)</span>
                  )}
                  {step.reason && (
                    <span className="ml-1 text-muted-foreground">
                      — {step.reason}
                    </span>
                  )}
                </span>
              </li>
            ))}
          </ul>

          {plan.replanReason && !plan.completed && (
            <div className="mt-1.5 text-[11px] text-muted-foreground">
              <span aria-hidden>🔁 </span>
              Replan: {plan.replanReason}
            </div>
          )}
          {plan.completed && plan.summary && (
            <div className="mt-1.5 text-[11px] text-muted-foreground">
              <span aria-hidden>✅ </span>
              {plan.summary}
            </div>
          )}
        </div>

        <span className="mt-1 shrink-0 text-[10px] tabular-nums text-muted-foreground/50">
          #{item.seq}
        </span>
      </div>
    </div>
  );
}

function truncate(s: string): string {
  if (s.length <= DETAIL_MAX_CHARS) return s;
  return s.slice(0, DETAIL_MAX_CHARS) + "\n... (truncated)";
}

/**
 * Compact a token count for the footer (`12500 → "12.5k"`,
 * `1_200_000 → "1.2M"`). Numbers below 1k render raw.
 */
function compactTokens(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) {
    const v = n / 1000;
    return v >= 100 ? `${Math.round(v)}k` : `${v.toFixed(1)}k`;
  }
  const v = n / 1_000_000;
  return v >= 100 ? `${Math.round(v)}M` : `${v.toFixed(1)}M`;
}

/** Per-turn token footer: `↑input ↓output` in small muted text. */
function TokenFooterBody({ usage }: { usage: TurnUsage }) {
  return (
    <div className="ml-[72px] flex items-center gap-2 text-[10px] tabular-nums text-muted-foreground/70">
      <span aria-label={`prompt ${usage.input} tokens`}>
        ↑{compactTokens(usage.input)}
      </span>
      <span aria-label={`completion ${usage.output} tokens`}>
        ↓{compactTokens(usage.output)}
      </span>
    </div>
  );
}

/**
 * Horizontal chip list of currently-running background tasks with live
 * elapsed timers. A single 1Hz interval re-renders all chips while any
 * task is active; it's cleared on unmount (i.e. when the row is removed
 * after the active set goes empty).
 */
function BackgroundTasksBody({ tasks }: { tasks: BackgroundTaskInfo[] }) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (tasks.length === 0) return;
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [tasks.length]);

  const palette = KIND_PALETTE.background_tasks;
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      <span
        className={cn(
          "inline-flex min-w-[60px] shrink-0 items-center justify-center rounded px-1.5 py-0.5 text-[11px] font-medium",
          palette.label,
        )}
      >
        <Loader2 className="mr-1 h-3 w-3 shrink-0 animate-spin" />
        <span className="truncate">{palette.text}</span>
      </span>
      {tasks.map((t) => {
        const elapsed = Math.max(0, Math.floor((now - t.startedAt) / 1000));
        return (
          <span
            key={t.taskId}
            title={t.description || t.name}
            className={cn(
              "inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-[11px]",
              palette.label,
            )}
          >
            <span className="truncate max-w-[180px]">{t.name}</span>
            <span className="tabular-nums opacity-70">{elapsed}s</span>
          </span>
        );
      })}
    </div>
  );
}
