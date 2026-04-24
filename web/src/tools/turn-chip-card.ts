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
 * Lit template for the per-turn tool-call chip card that lives below the
 * assistant's text reply. Mirrors `TurnToolcallCard.tsx` (used by the
 * kernel SessionDetail view) but rendered in Lit so it can be appended
 * inside the `<assistant-message>` host without a React↔Lit bridge.
 *
 * Local `<details>` elements keep the expand/collapse state in the DOM
 * so Lit's incremental re-render does not reset user interaction —
 * same trick pi-web-ui's `<code-block>` uses.
 */

import type { ToolCall, ToolResultMessage } from '@mariozechner/pi-ai';
import { html, type TemplateResult } from 'lit';

import type { ToolCallWithResult } from '@/pages/pi-chat-messages';

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

const PREVIEW_MAX = 110;
const DETAIL_MAX = 4000;

type ChipStatus = 'running' | 'completed' | 'errored';

interface ChipView {
  id: string;
  tool: string;
  preview: string;
  status: ChipStatus;
  errorText: string | undefined;
  inputJson: string | undefined;
  outputText: string | undefined;
}

function shortenPath(p: string): string {
  const parts = p.split('/').filter(Boolean);
  if (parts.length <= 2) return p;
  return '…/' + parts.slice(-2).join('/');
}

function clip(s: string): string {
  const flat = s.replace(/\s+/g, ' ').trim();
  return flat.length > PREVIEW_MAX ? flat.slice(0, PREVIEW_MAX) + '…' : flat;
}

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

function extractOutputText(result: ToolResultMessage | undefined): string {
  if (!result) return '';
  const parts: string[] = [];
  for (const block of result.content ?? []) {
    if ('text' in block && typeof block.text === 'string') parts.push(block.text);
  }
  return parts.join('\n');
}

function truncateDetail(s: string): string {
  if (s.length <= DETAIL_MAX) return s;
  return s.slice(0, DETAIL_MAX) + '\n... (truncated)';
}

function toChip(entry: ToolCallWithResult): ChipView {
  const { call, result } = entry;
  const args = (call.arguments ?? {}) as Record<string, unknown>;
  const outputText = extractOutputText(result);
  let status: ChipStatus = 'running';
  let errorText: string | undefined;
  if (result) {
    if (result.isError) {
      status = 'errored';
      if (outputText) errorText = clip(outputText);
    } else {
      status = 'completed';
    }
  }
  const inputJson = args && Object.keys(args).length > 0 ? safeStringify(args) : undefined;
  const view: ChipView = {
    id: call.id,
    tool: call.name,
    preview: derivePreview(args),
    status,
    errorText,
    inputJson,
    outputText: outputText.length > 0 ? truncateDetail(outputText) : undefined,
  };
  return view;
}

function safeStringify(v: unknown): string {
  try {
    return JSON.stringify(v, null, 2);
  } catch {
    return String(v);
  }
}

/**
 * Build a Lit template for the chip card attached to one assistant
 * turn. Returns `null` when there are no tool calls to display so the
 * caller can skip appending empty chrome.
 */
export function renderTurnChipCard(
  entries: readonly ToolCallWithResult[],
  opts: { isLive: boolean },
): TemplateResult | null {
  if (entries.length === 0) return null;
  const chips = entries.map(toChip);
  const completed = chips.filter((c) => c.status === 'completed').length;
  const errored = chips.filter((c) => c.status === 'errored').length;
  const running = chips.filter((c) => c.status === 'running').length;

  return html`
    <details
      class="rara-turn-chipcard group mt-2 rounded-md border border-border/40 bg-muted/20 px-2 py-1.5"
      open
    >
      <summary class="flex cursor-pointer list-none items-center gap-2 marker:hidden">
        <span class="text-xs font-medium text-foreground">
          ${chips.length} tool call${chips.length === 1 ? '' : 's'}
        </span>
        <span class="flex-1 truncate text-[11px] text-muted-foreground">
          ${running > 0 ? html`<span class="mr-2">${running} running</span>` : ''}
          ${completed > 0 ? html`<span class="mr-2 text-emerald-600">${completed} ok</span>` : ''}
          ${errored > 0 ? html`<span class="mr-2 text-destructive">${errored} errored</span>` : ''}
        </span>
        ${opts.isLive
          ? html`
              <span
                class="h-1.5 w-1.5 shrink-0 animate-pulse rounded-full bg-emerald-500"
                aria-hidden
              ></span>
            `
          : ''}
        <span class="rara-chipcard-chevron shrink-0 text-muted-foreground transition-transform"
          >▾</span
        >
      </summary>
      <div class="mt-1.5 flex flex-col gap-1">${chips.map((c) => renderChip(c))}</div>
    </details>
  `;
}

function renderChip(chip: ChipView): TemplateResult {
  const body = chip.status === 'errored' && chip.errorText ? chip.errorText : chip.preview;
  const hasDetail = chip.inputJson !== undefined || chip.outputText !== undefined;
  const row = html`
    <div
      class="flex items-center gap-2 rounded-sm bg-muted/40 px-2 py-1 text-[11px] ${hasDetail
        ? 'cursor-pointer hover:bg-muted/60'
        : ''}"
    >
      ${hasDetail ? html`<span class="shrink-0 text-muted-foreground/60">▸</span>` : ''}
      ${statusIcon(chip.status)}
      <span class="shrink-0 font-mono text-foreground">${chip.tool}</span>
      ${body
        ? html`<span class="min-w-0 flex-1 truncate text-muted-foreground">${body}</span>`
        : ''}
    </div>
  `;
  if (!hasDetail) return row;
  return html`
    <details class="rara-chip-detail">
      <summary class="list-none marker:hidden">${row}</summary>
      <div class="mt-1 rounded border bg-muted/40">
        ${chip.inputJson
          ? html`
              <pre
                class="max-h-60 overflow-auto whitespace-pre-wrap break-all p-3 text-[11px] text-muted-foreground"
              >
${chip.inputJson}</pre
              >
            `
          : ''}
        ${chip.outputText
          ? html`
              <pre
                class="max-h-60 overflow-auto whitespace-pre-wrap break-all border-t p-3 text-[11px] text-muted-foreground"
              >
${chip.outputText}</pre
              >
            `
          : ''}
      </div>
    </details>
  `;
}

function statusIcon(status: ChipStatus): TemplateResult {
  if (status === 'running') {
    return html`
      <span
        role="status"
        aria-label="running"
        class="inline-block h-3 w-3 shrink-0 animate-spin rounded-full border border-muted-foreground/30 border-t-muted-foreground"
      ></span>
    `;
  }
  if (status === 'completed') {
    return html`<span
      aria-label="completed"
      class="inline-block h-3 w-3 shrink-0 rounded-full bg-emerald-500"
    ></span>`;
  }
  return html`<span
    aria-label="errored"
    class="inline-block h-3 w-3 shrink-0 rounded-full bg-destructive"
  ></span>`;
}

// Consumed by callers to satisfy stricter "unused" type flags — actually
// re-exported so the chip model stays discoverable from the card module.
export type { ToolCall, ToolResultMessage };
