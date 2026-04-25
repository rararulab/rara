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

/**
 * Shared tool-chip model and presentation used by both the live
 * `SingleAgentLiveCard` and the historical `TurnToolcallCard`.
 *
 * A "chip" folds a `tool_use` timeline item together with its paired
 * `tool_result` (or error) into one compact row showing the tool name,
 * a short preview of the input, and a status icon.
 */

import { AlertCircle, CheckCircle2, ChevronRight } from 'lucide-react';
import { useState } from 'react';

import type { TimelineItem } from '@/api/kernel-types';
import { cn } from '@/lib/utils';

const DETAIL_MAX_CHARS = 4000;

/** Status of a single folded tool chip. */
export type ToolChipStatus = 'running' | 'completed' | 'errored';

/** Data backing a single folded tool chip. */
export interface ToolChipModel {
  /** Source `tool_use` seq — used as React key. */
  seq: number;
  tool: string;
  /** Derived short preview from the (redacted) input. Empty if nothing useful. */
  preview: string;
  status: ToolChipStatus;
  /** Short error text when `status === 'errored'`. Replaces the preview in the UI. */
  errorText?: string;
  /** Raw tool input kept for expandable detail views. */
  input?: Record<string, unknown> | undefined;
  /** Raw paired output (preview text) kept for expandable detail views. */
  output?: string | undefined;
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
 */
export function buildToolChips(items: readonly TimelineItem[]): ToolChipModel[] {
  const chips: ToolChipModel[] = [];
  for (let i = 0; i < items.length; i++) {
    const use = items[i];
    if (!use || use.kind !== 'tool_use' || !use.tool) continue;
    const toolName = use.tool;
    let status: ToolChipStatus = 'running';
    let errorText: string | undefined;
    let output: string | undefined;
    for (let j = i + 1; j < items.length; j++) {
      const follow = items[j];
      if (!follow) continue;
      if (follow.kind === 'tool_result' && follow.tool === toolName) {
        status = follow.success === false ? 'errored' : 'completed';
        if (follow.output) output = follow.output;
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
      input: use.input,
    };
    if (errorText !== undefined) chip.errorText = errorText;
    if (output !== undefined) chip.output = output;
    chips.push(chip);
  }
  return chips;
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

export interface ToolChipProps {
  chip: ToolChipModel;
  /** When true, the chip is a button that expands an input/output detail pane. */
  expandable?: boolean;
}

/**
 * Compact one-line chip: status icon · tool name · preview (or error text).
 *
 * When `expandable` is set, clicking the chip toggles a muted detail panel
 * showing the full tool input JSON and (when available) the paired output.
 */
export function ToolChip({ chip, expandable = false }: ToolChipProps) {
  const [expanded, setExpanded] = useState(false);
  const body = chip.status === 'errored' && chip.errorText ? chip.errorText : chip.preview;
  const hasDetail =
    (!!chip.input && Object.keys(chip.input).length > 0) || !!(chip.output && chip.output.length);
  const canExpand = expandable && hasDetail;

  const row = (
    <div
      className={cn(
        'flex items-center gap-2 rounded-sm bg-muted/40 px-2 py-1 text-[11px]',
        canExpand && 'cursor-pointer hover:bg-muted/60',
      )}
    >
      {canExpand && (
        <ChevronRight
          className={cn(
            'h-3 w-3 shrink-0 text-muted-foreground/50 transition-transform',
            expanded && 'rotate-90',
          )}
          aria-hidden
        />
      )}
      <StatusIcon status={chip.status} />
      <span className="shrink-0 font-mono text-foreground">{chip.tool}</span>
      {body && <span className="min-w-0 flex-1 truncate text-muted-foreground">{body}</span>}
    </div>
  );

  if (!canExpand) {
    return row;
  }

  return (
    <div>
      <button
        type="button"
        onClick={() => setExpanded((v) => !v)}
        className="block w-full text-left"
        aria-expanded={expanded}
      >
        {row}
      </button>
      <div
        className={cn(
          'grid transition-[grid-template-rows] duration-200 ease-out',
          expanded ? 'grid-rows-[1fr]' : 'grid-rows-[0fr]',
        )}
      >
        <div className="overflow-hidden">
          <div className="mt-1 rounded border bg-muted/40">
            {chip.input && Object.keys(chip.input).length > 0 && (
              <pre className="max-h-60 overflow-auto whitespace-pre-wrap break-all p-3 text-[11px] text-muted-foreground">
                {JSON.stringify(chip.input, null, 2)}
              </pre>
            )}
            {chip.output && chip.output.length > 0 && (
              <pre className="max-h-60 overflow-auto whitespace-pre-wrap break-all border-t p-3 text-[11px] text-muted-foreground">
                {truncate(formatMaybeJson(chip.output))}
              </pre>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

function StatusIcon({ status }: { status: ToolChipStatus }) {
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

function truncate(s: string): string {
  if (s.length <= DETAIL_MAX_CHARS) return s;
  return s.slice(0, DETAIL_MAX_CHARS) + '\n... (truncated)';
}

/**
 * Pretty-print tool output when it's a JSON document, leave it untouched
 * otherwise. Tool outputs arrive as opaque strings — most kernel tools return
 * a JSON payload, but some return Markdown / plain text we must not reformat.
 */
export function formatMaybeJson(s: string): string {
  const trimmed = s.trim();
  if (!trimmed.startsWith('{') && !trimmed.startsWith('[')) return s;
  try {
    return JSON.stringify(JSON.parse(trimmed), null, 2);
  } catch {
    return s;
  }
}
