/*
 * Copyright 2025 Rararulab
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 */

/**
 * A compact tool renderer inspired by multica's `ToolCallRow`
 * (vendor/multica/packages/views/chat/components/chat-message-list.tsx).
 *
 * pi-web-ui's `DefaultRenderer` is visually heavy: two labeled JSON code
 * blocks (Input/Output) plus a header, per tool call. That dominates
 * the chat column when an assistant turn chains several tools. This
 * renderer compresses each call into a single-line summary and hides
 * the JSON payloads inside a native `<details>` element.
 *
 * Why `<details>` instead of pi-web-ui's `renderCollapsibleHeader`:
 * tool renderers are re-invoked on every stream tick (params stream in
 * one JSON chunk, then the tool result arrives). `renderCollapsibleHeader`
 * encodes open/closed state as a CSS class on a host div that Lit
 * rewrites on every render — so a user's expansion would collapse on
 * the next token. `<details>` keeps `open` as DOM state that survives
 * Lit's incremental re-render, matching what pi-web-ui's own
 * `<code-block>` relies on.
 */

import type { ToolResultMessage } from '@mariozechner/pi-ai';
import { renderHeader, type ToolRenderer, type ToolRenderResult } from '@mariozechner/pi-web-ui';
import { html } from 'lit';
import { Wrench } from 'lucide';

const MAX_SUMMARY = 120;
const MAX_RESULT_EXPANDED = 4000;

/**
 * Shorten a path to `.../parent/file` when it has more than two
 * segments. Matches multica's `shortenPath` so summaries read the same
 * across apps.
 */
function shortenPath(p: string): string {
  const parts = p.split('/');
  if (parts.length <= 3) return p;
  return '.../' + parts.slice(-2).join('/');
}

/**
 * Collapse whitespace to single spaces and truncate. Summaries live in
 * a single-line `truncate` span, so unnormalised newlines (heredocs,
 * multi-line commands) would either wrap awkwardly or break layout.
 */
function normalizeAndTruncate(s: string, n: number): string {
  const flat = s.replace(/\s+/g, ' ').trim();
  return flat.length > n ? flat.slice(0, n) + '…' : flat;
}

/**
 * Pull a human-readable one-liner out of a tool's input JSON. Field
 * priority mirrors multica's `getToolSummary`.
 */
function summarizeParams(params: unknown): string {
  if (!params || typeof params !== 'object') return '';
  const inp = params as Record<string, unknown>;

  const pick = (k: string): string | null => {
    const v = inp[k];
    return typeof v === 'string' && v.length > 0 ? v : null;
  };

  const command = pick('command');
  if (command) return normalizeAndTruncate(command, MAX_SUMMARY);

  const query = pick('query') ?? pick('pattern');
  if (query) return normalizeAndTruncate(query, MAX_SUMMARY);

  const filePath = pick('file_path') ?? pick('path');
  if (filePath) return shortenPath(filePath);

  const description = pick('description');
  if (description) return normalizeAndTruncate(description, MAX_SUMMARY);

  const prompt = pick('prompt');
  if (prompt) return normalizeAndTruncate(prompt, MAX_SUMMARY);

  const skill = pick('skill');
  if (skill) return skill;

  for (const v of Object.values(inp)) {
    if (typeof v === 'string' && v.length > 0) {
      return normalizeAndTruncate(v, MAX_SUMMARY);
    }
  }
  return '';
}

function parseParams(raw: unknown): unknown {
  if (raw == null) return undefined;
  if (typeof raw === 'string') {
    try {
      return JSON.parse(raw);
    } catch {
      return raw;
    }
  }
  return raw;
}

function formatJson(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

interface ExtractedOutput {
  text: string;
  nonTextCount: number;
}

function extractOutput(result: ToolResultMessage | undefined): ExtractedOutput {
  if (!result) return { text: '', nonTextCount: 0 };
  const textParts: string[] = [];
  let nonTextCount = 0;
  for (const block of result.content ?? []) {
    if ('text' in block) {
      textParts.push(block.text ?? '');
    } else {
      // Currently only ImageContent falls here; treated uniformly so
      // we degrade gracefully if pi-ai grows new block types.
      void block;
      nonTextCount += 1;
    }
  }
  return { text: textParts.join('\n'), nonTextCount };
}

export class CompactToolRenderer implements ToolRenderer {
  private readonly toolName: string;

  constructor(toolName: string) {
    this.toolName = toolName;
  }

  render(
    params: unknown,
    result: ToolResultMessage | undefined,
    isStreaming?: boolean,
  ): ToolRenderResult {
    const state: 'inprogress' | 'complete' | 'error' = result
      ? result.isError
        ? 'error'
        : 'complete'
      : isStreaming
        ? 'inprogress'
        : 'complete';

    const parsed = parseParams(params);
    const summary = summarizeParams(parsed);
    const { text: output, nonTextCount } = extractOutput(result);

    const headerLabel = html`
      <span class="flex items-center gap-2 min-w-0 overflow-hidden">
        <span class="font-medium text-foreground shrink-0">${this.toolName}</span>
        ${summary ? html`<span class="truncate text-muted-foreground">${summary}</span>` : ''}
      </span>
    `;

    const hasParams = parsed !== undefined;
    const hasOutput = output.length > 0 || nonTextCount > 0;

    // Nothing to expand — render plain header (streaming "thinking"
    // state before the first param token arrives).
    if (!hasParams && !hasOutput) {
      return {
        content: renderHeader(state, Wrench, headerLabel),
        isCustom: false,
      };
    }

    const paramsJson = hasParams ? formatJson(parsed) : '';
    const outputBody =
      output.length > MAX_RESULT_EXPANDED
        ? output.slice(0, MAX_RESULT_EXPANDED) + '\n… (truncated)'
        : output;
    const outputPlaceholder =
      !output && nonTextCount > 0
        ? `(no text output — ${nonTextCount} non-text block${nonTextCount === 1 ? '' : 's'})`
        : '';

    return {
      content: html`
        <details class="group">
          <summary class="list-none cursor-pointer marker:hidden">
            ${renderHeader(state, Wrench, headerLabel)}
          </summary>
          <div class="mt-2 space-y-2">
            ${paramsJson
              ? html`
                  <div>
                    <div class="text-[11px] font-medium mb-1 text-muted-foreground">Input</div>
                    <pre
                      class="max-h-40 overflow-auto rounded bg-muted/50 p-2 text-[11px] text-muted-foreground whitespace-pre-wrap break-all"
                    >
${paramsJson}</pre
                    >
                  </div>
                `
              : ''}
            ${hasOutput
              ? html`
                  <div>
                    <div class="text-[11px] font-medium mb-1 text-muted-foreground">Output</div>
                    <pre
                      class="max-h-60 overflow-auto rounded bg-muted/50 p-2 text-[11px] text-muted-foreground whitespace-pre-wrap break-all"
                    >
${outputBody || outputPlaceholder}</pre
                    >
                  </div>
                `
              : ''}
          </div>
        </details>
      `,
      isCustom: false,
    };
  }
}
