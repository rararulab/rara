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
 * Per-tool rich content renderers for the new chat UI (`/chat-v2`).
 *
 * Layout strategy:
 * - The expanded body of a `<Tool>` card delegates to a per-tool component
 *   that knows how to render that tool's input + output cleanly. Bash
 *   commands show the command + stdout/stderr, generic fallback dumps the
 *   raw JSON. Header summaries (e.g. "bash · ls -la") are returned as a
 *   short string so the caller can put them inline in the collapsed
 *   header.
 * - Renderers are pure presentational — they never mutate the part and
 *   tolerate partial inputs (mid-stream the `output` may be undefined).
 */

import type { DynamicToolUIPart } from 'ai';
import type { ReactNode } from 'react';

import { CodeBlock } from './ai-elements/code-block';
import { ToolInput, ToolOutput } from './ai-elements/tool';

/** Tool names that we treat as "shell-like" — the input has a `command`
 *  field and the output is plain stdout/stderr text. */
const BASH_TOOL_NAMES = new Set(['bash', 'shell', 'exec', 'run_command']);

/** Extract the executed command from a bash-like tool's `input`. The
 *  contract across rara's bash tool variants is `{ command: string }`,
 *  but we defensively probe a few aliases so a renamed payload doesn't
 *  silently render an empty header. */
function extractCommand(input: unknown): string | null {
  if (!input || typeof input !== 'object') return null;
  const obj = input as Record<string, unknown>;
  for (const key of ['command', 'cmd', 'shell', 'script']) {
    const v = obj[key];
    if (typeof v === 'string' && v.length > 0) return v;
  }
  return null;
}

/** Best-effort string coercion of a tool output. Tool outputs land as
 *  either a stringified preview (live stream) or a structured block
 *  (history reload). */
function outputToText(output: unknown): string {
  if (output == null) return '';
  if (typeof output === 'string') return output;
  try {
    return JSON.stringify(output, null, 2);
  } catch {
    return String(output);
  }
}

export type ToolRendererProps = {
  part: DynamicToolUIPart;
};

/** Inline header summary appended after the tool name, e.g. the bash
 *  command. Returns `null` for tools that don't have a useful one-liner.
 *
 *  Truncation is left to CSS — wrapping containers should clip with
 *  `truncate` so a long command doesn't push the status badge off-screen. */
export function toolHeaderSummary(part: DynamicToolUIPart): string | null {
  if (BASH_TOOL_NAMES.has(part.toolName)) {
    return extractCommand(part.input);
  }
  return null;
}

/** Pick the renderer for a given tool part. Falls through to the generic
 *  JSON dump for anything we don't recognise. */
export function ToolRenderer({ part }: ToolRendererProps): ReactNode {
  if (BASH_TOOL_NAMES.has(part.toolName)) {
    return <BashRenderer part={part} />;
  }
  return <GenericRenderer part={part} />;
}

/** Bash / shell / exec renderer. Shows the command in a single mono pre,
 *  then stdout/stderr (or error) below it. */
function BashRenderer({ part }: ToolRendererProps): ReactNode {
  const command = extractCommand(part.input);
  const errorText = part.state === 'output-error' ? part.errorText : undefined;
  const outputText = part.state === 'output-available' ? outputToText(part.output) : undefined;

  return (
    <div className="space-y-3">
      {command && (
        <div className="space-y-2">
          <h4 className="font-medium text-muted-foreground text-xs uppercase tracking-wide">
            Command
          </h4>
          <CodeBlock code={command} language="bash" />
        </div>
      )}
      {(outputText || errorText) && (
        <div className="space-y-2">
          <h4 className="font-medium text-muted-foreground text-xs uppercase tracking-wide">
            {errorText ? 'Error' : 'Output'}
          </h4>
          <div
            className={
              errorText
                ? 'overflow-x-auto rounded-md bg-destructive/10 p-3 font-mono text-xs text-destructive whitespace-pre-wrap'
                : 'overflow-x-auto rounded-md bg-muted/50 p-3 font-mono text-xs whitespace-pre-wrap'
            }
          >
            {errorText ?? outputText}
          </div>
        </div>
      )}
    </div>
  );
}

/** Generic fallback: reuse the ai-elements `ToolInput` + `ToolOutput`
 *  components — they JSON-pretty the input and stringify the output. */
function GenericRenderer({ part }: ToolRendererProps): ReactNode {
  const errorText = part.state === 'output-error' ? part.errorText : undefined;
  const output = part.state === 'output-available' ? part.output : undefined;
  return (
    <>
      <ToolInput input={part.input} />
      <ToolOutput output={output} errorText={errorText} />
    </>
  );
}
