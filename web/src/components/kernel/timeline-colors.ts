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

import type { EventKind } from "@/api/kernel-types";

/** Visual classes for one timeline event-kind, across 3 use sites. */
export interface KindPalette {
  /** Muted bar segment (TimelineBar default). */
  bar: string;
  /** Saturated bar segment (TimelineBar selected). */
  barActive: string;
  /** Pill for the row's type-label badge. */
  label: string;
  /** Short textual label rendered inside the badge. */
  text: string;
}

/**
 * Color palette per {@link EventKind}.
 *
 * Mirrors multica's agent transcript colors: emerald / violet / blue /
 * slate / red. Semantic tokens (`bg-info`, `bg-destructive`) are not used
 * here because we deliberately want 5 distinct hues — semantic tokens
 * collapse to 2-3 colors.
 */
export const KIND_PALETTE: Record<EventKind, KindPalette> = {
  agent: {
    bar: "bg-emerald-400/60",
    barActive: "bg-emerald-500",
    label:
      "bg-emerald-500/20 text-emerald-700 dark:bg-emerald-500/15 dark:text-emerald-300",
    text: "Agent",
  },
  thinking: {
    bar: "bg-violet-400/60",
    barActive: "bg-violet-500",
    label:
      "bg-violet-500/20 text-violet-700 dark:bg-violet-500/15 dark:text-violet-300",
    text: "Think",
  },
  tool_use: {
    bar: "bg-blue-400/60",
    barActive: "bg-blue-500",
    label:
      "bg-blue-500/20 text-blue-700 dark:bg-blue-500/15 dark:text-blue-300",
    text: "Tool",
  },
  tool_result: {
    bar: "bg-slate-300/60 dark:bg-slate-600/60",
    barActive: "bg-slate-400 dark:bg-slate-500",
    label: "bg-muted text-muted-foreground",
    text: "Result",
  },
  error: {
    bar: "bg-red-400/60",
    barActive: "bg-red-500",
    label:
      "bg-red-500/20 text-red-700 dark:bg-red-500/15 dark:text-red-300",
    text: "Error",
  },
  in_progress: {
    // Reuse the thinking violet hue so the placeholder visually belongs
    // to the "pre-output" phase of the turn.
    bar: "bg-violet-300/50",
    barActive: "bg-violet-400",
    label:
      "bg-violet-500/15 text-violet-700 dark:bg-violet-500/10 dark:text-violet-300",
    text: "…",
  },
};

/** Short label to show inside a row's type badge. */
export function eventLabel(kind: EventKind, tool?: string): string {
  if (kind === "tool_use" || kind === "tool_result") {
    return tool ?? KIND_PALETTE[kind].text;
  }
  return KIND_PALETTE[kind].text;
}

/**
 * One-line summary for a row — picked from the most informative field on
 * the item. Truncated at the call site via CSS, not here.
 */
export function eventSummary(item: {
  kind: EventKind;
  content?: string;
  input?: Record<string, unknown>;
  output?: string;
}): string {
  switch (item.kind) {
    case "agent":
    case "thinking":
    case "error":
    case "in_progress":
      return item.content?.trim() ?? "";
    case "tool_use":
      return toolInputSummary(item.input);
    case "tool_result":
      return item.output?.trim().slice(0, 200) ?? "";
  }
}

/**
 * Extract a short human-readable description from tool arguments.
 *
 * Picks the most-informative string field in priority order
 * (query / file_path / pattern / command / prompt / skill). Returns
 * empty string when nothing suitable is found.
 */
function toolInputSummary(input?: Record<string, unknown>): string {
  if (!input) return "";
  const keys = [
    "query",
    "file_path",
    "path",
    "pattern",
    "description",
    "command",
    "prompt",
    "skill",
  ];
  for (const k of keys) {
    const v = input[k];
    if (typeof v === "string" && v.length > 0) {
      if (k === "file_path" || k === "path") return shortenPath(v);
      if (v.length > 120) return v.slice(0, 120) + "...";
      return v;
    }
  }
  for (const v of Object.values(input)) {
    if (typeof v === "string" && v.length > 0 && v.length < 120) return v;
  }
  return "";
}

function shortenPath(p: string): string {
  const parts = p.split("/");
  if (parts.length <= 3) return p;
  return ".../" + parts.slice(-2).join("/");
}
