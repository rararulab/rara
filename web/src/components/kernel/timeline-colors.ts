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

import type { EventKind } from '@/api/kernel-types';

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
    bar: 'bg-emerald-400/60',
    barActive: 'bg-emerald-500',
    label: 'bg-emerald-500/20 text-emerald-700 dark:bg-emerald-500/15 dark:text-emerald-300',
    text: 'Agent',
  },
  thinking: {
    bar: 'bg-violet-400/60',
    barActive: 'bg-violet-500',
    label: 'bg-violet-500/20 text-violet-700 dark:bg-violet-500/15 dark:text-violet-300',
    text: 'Think',
  },
  tool_use: {
    bar: 'bg-blue-400/60',
    barActive: 'bg-blue-500',
    label: 'bg-blue-500/20 text-blue-700 dark:bg-blue-500/15 dark:text-blue-300',
    text: 'Tool',
  },
  tool_result: {
    bar: 'bg-slate-300/60 dark:bg-slate-600/60',
    barActive: 'bg-slate-400 dark:bg-slate-500',
    label: 'bg-muted text-muted-foreground',
    text: 'Result',
  },
  error: {
    bar: 'bg-red-400/60',
    barActive: 'bg-red-500',
    label: 'bg-red-500/20 text-red-700 dark:bg-red-500/15 dark:text-red-300',
    text: 'Error',
  },
  in_progress: {
    // Reuse the thinking violet hue so the placeholder visually belongs
    // to the "pre-output" phase of the turn.
    bar: 'bg-violet-300/50',
    barActive: 'bg-violet-400',
    label: 'bg-violet-500/15 text-violet-700 dark:bg-violet-500/10 dark:text-violet-300',
    text: '…',
  },
  plan_card: {
    // Amber distinguishes "directive / structured plan" from the other
    // kinds while staying within the existing tonal range.
    bar: 'bg-amber-400/60',
    barActive: 'bg-amber-500',
    label: 'bg-amber-500/20 text-amber-700 dark:bg-amber-500/15 dark:text-amber-300',
    text: 'Plan',
  },
  token_footer: {
    // Muted — the footer is metadata, not a distinct event-kind worth
    // its own hue. Reuse the result palette so TimelineBar renders a
    // slate segment when one appears.
    bar: 'bg-slate-300/40 dark:bg-slate-600/40',
    barActive: 'bg-slate-400 dark:bg-slate-500',
    label: 'bg-muted text-muted-foreground',
    text: 'Usage',
  },
  background_tasks: {
    // Cyan reads as "ambient / side-effect" — distinct from tool_use
    // blue so concurrent background work doesn't blend into tool calls.
    bar: 'bg-cyan-400/60',
    barActive: 'bg-cyan-500',
    label: 'bg-cyan-500/20 text-cyan-700 dark:bg-cyan-500/15 dark:text-cyan-300',
    text: 'BG',
  },
};

/** Short label to show inside a row's type badge. */
export function eventLabel(kind: EventKind, tool?: string): string {
  if (kind === 'tool_use' || kind === 'tool_result') {
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
  content?: string | undefined;
  input?: Record<string, unknown> | undefined;
  output?: string | undefined;
  plan?: { goal: string } | undefined;
}): string {
  switch (item.kind) {
    case 'agent':
    case 'thinking':
    case 'error':
    case 'in_progress':
      return item.content?.trim() ?? '';
    case 'tool_use':
      return toolSummary(item.input);
    case 'tool_result':
      return item.output?.trim().slice(0, 200) ?? '';
    case 'plan_card':
      return item.plan?.goal ?? '';
    case 'token_footer':
    case 'background_tasks':
      // Self-rendered rows — the badge+summary layout is bypassed in
      // TimelineRow, so this branch is unreachable in practice.
      return '';
  }
}

/**
 * Extract a short human-readable description from tool arguments.
 *
 * Spec priority (issue #1615): `description` → `query` →
 * `file_path`/`path` (shortened) → `command`. Falls through to legacy
 * kernel-timeline keys (`pattern`, `prompt`, `skill`) and finally any
 * non-trivial string value, so the kernel Timeline panel keeps surfacing
 * something useful for tools that don't carry any of the spec keys.
 *
 * `description` is allowed a slightly higher cap (120) because it tends
 * to be a short human sentence; everything else is capped at 100.
 *
 * Exported for the agent-live card so both surfaces share one source of
 * truth.
 */
export function toolSummary(input: Record<string, unknown> | null | undefined): string {
  if (!input) return '';

  const description = readString(input, 'description');
  if (description) return cap(description, 120);

  const query = readString(input, 'query');
  if (query) return cap(query, 100);

  const filePath = readString(input, 'file_path') ?? readString(input, 'path');
  if (filePath) return shortenPath(filePath);

  const command = readString(input, 'command');
  if (command) return cap(command, 100);

  // Legacy fallbacks for kernel tools that don't surface the spec keys.
  for (const k of ['pattern', 'prompt', 'skill']) {
    const v = readString(input, k);
    if (v) return cap(v, 100);
  }
  for (const v of Object.values(input)) {
    if (typeof v === 'string' && v.length > 0 && v.length < 120) return v;
  }
  return '';
}

function readString(obj: Record<string, unknown>, key: string): string | null {
  const v = obj[key];
  return typeof v === 'string' && v.length > 0 ? v : null;
}

function cap(s: string, max: number): string {
  return s.length > max ? `${s.slice(0, max)}...` : s;
}

function shortenPath(p: string): string {
  const parts = p.split('/');
  if (parts.length <= 3) return p;
  return '.../' + parts.slice(-2).join('/');
}
