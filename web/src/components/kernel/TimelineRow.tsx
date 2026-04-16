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

import { forwardRef, useState } from "react";
import { AlertCircle, Brain, ChevronRight } from "lucide-react";
import { cn } from "@/lib/utils";
import type { TimelineItem } from "@/api/kernel-types";
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
  }
}

function truncate(s: string): string {
  if (s.length <= DETAIL_MAX_CHARS) return s;
  return s.slice(0, DETAIL_MAX_CHARS) + "\n... (truncated)";
}
