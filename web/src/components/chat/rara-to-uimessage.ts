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
 * Adapter: rara WebSocket events / REST history → AI SDK `UIMessage` shape.
 *
 * The new chat shell (`PiChatV2`) consumes `UIMessage[]`, but rara's backend
 * speaks its own `PublicWebEvent` discriminated union (see
 * `@/adapters/rara-stream`) and a separate REST history shape
 * (`ChatMessageData` from `@/api/types`). This module bridges both feeds so
 * the React layer never has to fork pi-web-ui's internals.
 *
 * The reducer is intentionally pure: no fetch, no timers, no DOM access. The
 * caller owns the message list state and applies events one at a time.
 */

import type { DynamicToolUIPart, ReasoningUIPart, TextUIPart, UIMessage } from 'ai';

import type { PublicWebEvent } from '@/adapters/rara-stream';
import type { ChatContentBlock, ChatMessageData, ChatToolCallData } from '@/api/types';

/** Parts a rara assistant `UIMessage` can carry. */
type AssistantPart = TextUIPart | ReasoningUIPart | DynamicToolUIPart;

/** All parts a rara `UIMessage` can carry — user messages are text-only. */
type RaraPart = TextUIPart | ReasoningUIPart | DynamicToolUIPart;

/** Stable id generator. UIMessage requires unique ids; rara messages are
 *  sequenced by `seq` for history, and synthesised for live streaming. */
let liveCounter = 0;
function nextLiveId(prefix: string): string {
  liveCounter += 1;
  return `${prefix}-${Date.now()}-${liveCounter}`;
}

/** Track variants we have already warned about so a noisy stream doesn't
 *  spam the console. Module-scope is fine — the set is bounded by the
 *  WebEvent discriminated union. */
const warnedVariants = new Set<string>();

function warnUnknown(variant: string): void {
  if (warnedVariants.has(variant)) return;
  warnedVariants.add(variant);
  console.warn(`[rara-to-uimessage] unhandled WebEvent variant: ${variant}`);
}

// ---------------------------------------------------------------------------
// REST history → UIMessage[]
// ---------------------------------------------------------------------------

/**
 * Extract a plain-text rendering of a `ChatContentBlock[]` payload. Image /
 * audio / file blocks are flattened to a placeholder so the user still sees
 * something rendered while the rich-attachment path is deferred to a later
 * PR.
 */
function blocksToText(content: string | ChatContentBlock[]): string {
  if (typeof content === 'string') return content;
  return content
    .map((block) => {
      switch (block.type) {
        case 'text':
          return block.text;
        case 'image_url':
          return `![image](${block.url})`;
        case 'image_base64':
          return `[image: ${block.media_type}]`;
        case 'audio_base64':
          return `[audio: ${block.media_type}]`;
        case 'file_base64':
          return `[file: ${block.filename ?? block.media_type}]`;
        default:
          return '';
      }
    })
    .filter((s) => s.length > 0)
    .join('\n');
}

/**
 * Convert a single tool-call entry persisted on an assistant message into a
 * `dynamic-tool` part. The rara history endpoint does not return tool
 * outputs alongside the call — outputs arrive as separate `tool` /
 * `tool_result` messages. The caller stitches them together in
 * {@link historyToUIMessages}.
 */
function toolCallToPart(call: ChatToolCallData): DynamicToolUIPart {
  return {
    type: 'dynamic-tool',
    toolName: call.name,
    toolCallId: call.id,
    state: 'input-available',
    input: call.arguments,
  };
}

/**
 * Convert the REST `/api/v1/chat/sessions/{key}/messages` payload into the
 * `UIMessage[]` the new chat components consume.
 *
 * History rows are already in chronological order. Tool results follow the
 * assistant message that requested them; we resolve each result onto the
 * matching `dynamic-tool` part by `toolCallId`.
 */
export function historyToUIMessages(history: ChatMessageData[]): UIMessage[] {
  const messages: UIMessage[] = [];
  // Index of the assistant message holding each pending tool call so we can
  // resolve a later `tool` / `tool_result` row in-place.
  const toolCallIndex = new Map<string, { msg: number; part: number }>();

  for (const row of history) {
    if (row.role === 'system') {
      // System prompts are a server-side concern — they do not render in
      // the timeline. Skip without warning.
      continue;
    }

    if (row.role === 'user') {
      messages.push({
        id: `msg-${row.seq}`,
        role: 'user',
        parts: [{ type: 'text', text: blocksToText(row.content), state: 'done' }],
      });
      continue;
    }

    if (row.role === 'assistant') {
      const parts: AssistantPart[] = [];
      const text = blocksToText(row.content);
      if (text.length > 0) {
        parts.push({ type: 'text', text, state: 'done' });
      }
      if (row.tool_calls) {
        for (const call of row.tool_calls) {
          parts.push(toolCallToPart(call));
        }
      }
      const msgIdx = messages.length;
      messages.push({ id: `msg-${row.seq}`, role: 'assistant', parts });
      // Register tool-call slots so a subsequent `tool` / `tool_result` row
      // can attach its output.
      parts.forEach((part, partIdx) => {
        if (part.type === 'dynamic-tool') {
          toolCallIndex.set(part.toolCallId, { msg: msgIdx, part: partIdx });
        }
      });
      continue;
    }

    if (row.role === 'tool' || row.role === 'tool_result') {
      const id = row.tool_call_id;
      if (!id) continue;
      const slot = toolCallIndex.get(id);
      if (!slot) continue;
      const msg = messages[slot.msg];
      if (!msg) continue;
      const part = msg.parts[slot.part] as DynamicToolUIPart | undefined;
      if (!part || part.type !== 'dynamic-tool') continue;
      // Replace the part with its resolved form. We strip any narrowing
      // discriminant fields that the prior state had (none beyond `state`).
      msg.parts[slot.part] = {
        type: 'dynamic-tool',
        toolName: part.toolName,
        toolCallId: part.toolCallId,
        state: 'output-available',
        input: part.input,
        output: blocksToText(row.content),
      };
      toolCallIndex.delete(id);
    }
  }

  return messages;
}

// ---------------------------------------------------------------------------
// Live stream reducer
// ---------------------------------------------------------------------------

/** Find or insert the tail assistant message we should append to. */
function ensureAssistantTail(messages: UIMessage[]): {
  msg: UIMessage;
  index: number;
  created: boolean;
} {
  const last = messages[messages.length - 1];
  if (last && last.role === 'assistant') {
    return { msg: last, index: messages.length - 1, created: false };
  }
  const next: UIMessage = {
    id: nextLiveId('assistant'),
    role: 'assistant',
    parts: [],
  };
  messages.push(next);
  return { msg: next, index: messages.length - 1, created: true };
}

/** Append text onto the trailing text part of an assistant message, creating
 *  a new text part if the last one is something else (e.g. a tool call). */
function appendText(msg: UIMessage, delta: string): void {
  const tail = msg.parts[msg.parts.length - 1] as RaraPart | undefined;
  if (tail && tail.type === 'text') {
    tail.text += delta;
    tail.state = 'streaming';
    return;
  }
  msg.parts.push({ type: 'text', text: delta, state: 'streaming' });
}

/** Append reasoning text similarly. */
function appendReasoning(msg: UIMessage, delta: string): void {
  const tail = msg.parts[msg.parts.length - 1] as RaraPart | undefined;
  if (tail && tail.type === 'reasoning') {
    tail.text += delta;
    tail.state = 'streaming';
    return;
  }
  msg.parts.push({ type: 'reasoning', text: delta, state: 'streaming' });
}

/** Mark every still-streaming text/reasoning part on the assistant tail as
 *  done. Called when the run finishes so the renderer can drop streaming
 *  affordances (cursors, shimmer, etc). */
function markAssistantDone(msg: UIMessage): void {
  for (const part of msg.parts) {
    if (part.type === 'text' || part.type === 'reasoning') {
      part.state = 'done';
    }
  }
}

/** Locate a `dynamic-tool` part by its tool-call id across the message list,
 *  searching backwards because the active call is almost always on the tail. */
function findToolCall(
  messages: UIMessage[],
  toolCallId: string,
): { msg: UIMessage; part: DynamicToolUIPart; index: number } | null {
  for (let i = messages.length - 1; i >= 0; i--) {
    const msg = messages[i];
    if (!msg || msg.role !== 'assistant') continue;
    for (let j = msg.parts.length - 1; j >= 0; j--) {
      const part = msg.parts[j];
      if (part && part.type === 'dynamic-tool' && part.toolCallId === toolCallId) {
        return { msg, part, index: j };
      }
    }
  }
  return null;
}

/**
 * Pure reducer: apply one `PublicWebEvent` to a `UIMessage[]` and return the
 * updated list. The function MUTATES message and part objects in-place but
 * always returns a fresh outer array so React `setState(prev => ...)` still
 * triggers a re-render.
 *
 * Variants we cannot map cleanly are logged (once) and skipped — never
 * thrown — so a stale frontend doesn't crash on a new backend variant.
 */
export function applyRaraEvent(messages: UIMessage[], event: PublicWebEvent): UIMessage[] {
  const next = [...messages];

  switch (event.type) {
    case '__stream_started':
    case '__stream_closed':
      // Lifecycle bookends. The caller may want these for connection state
      // but they do not change the message list.
      return next;

    case 'text_delta': {
      const { msg } = ensureAssistantTail(next);
      appendText(msg, event.text);
      return next;
    }

    case 'reasoning_delta': {
      const { msg } = ensureAssistantTail(next);
      appendReasoning(msg, event.text);
      return next;
    }

    case 'tool_call_start': {
      const { msg } = ensureAssistantTail(next);
      msg.parts.push({
        type: 'dynamic-tool',
        toolName: event.name,
        toolCallId: event.id,
        state: 'input-available',
        input: event.arguments,
      });
      return next;
    }

    case 'tool_call_end': {
      const found = findToolCall(next, event.id);
      if (!found) return next;
      const { msg, part, index } = found;
      msg.parts[index] = event.success
        ? {
            type: 'dynamic-tool',
            toolName: part.toolName,
            toolCallId: part.toolCallId,
            state: 'output-available',
            input: part.input,
            output: event.result_preview,
          }
        : {
            type: 'dynamic-tool',
            toolName: part.toolName,
            toolCallId: part.toolCallId,
            state: 'output-error',
            input: part.input,
            errorText: event.error ?? event.result_preview ?? 'tool error',
          };
      return next;
    }

    case 'message': {
      // One-shot complete message — rara collapses the whole turn into a
      // single frame. Treat it as a final text body on a fresh assistant.
      const { msg } = ensureAssistantTail(next);
      appendText(msg, event.content);
      markAssistantDone(msg);
      return next;
    }

    case 'done': {
      const tail = next[next.length - 1];
      if (tail && tail.role === 'assistant') markAssistantDone(tail);
      return next;
    }

    case 'error': {
      // Surface the error inline so the user sees what failed without
      // hunting in devtools. PR6 will polish the visual treatment.
      const { msg } = ensureAssistantTail(next);
      appendText(msg, `\n\n[error] ${event.message}`);
      markAssistantDone(msg);
      return next;
    }

    // Informational frames the renderer does not surface yet. Listed
    // explicitly so the exhaustiveness check below still works.
    case 'typing':
    case 'progress':
    case 'turn_rationale':
    case 'turn_metrics':
    case 'usage':
    case 'phase':
    case 'attachment':
    case 'approval_requested':
    case 'approval_resolved':
      // TODO(PR3+): surface tool attachments inline; surface approvals
      // through a UI affordance; render usage metadata in the header.
      return next;

    default: {
      // Exhaustiveness guard: if a new variant lands the type narrows to
      // `never` and TS errors at compile time. At runtime we log once and
      // move on.
      const _exhaustive: never = event;
      void _exhaustive;
      warnUnknown(JSON.stringify(event));
      return next;
    }
  }
}

/**
 * Fold a sequence of buffered events into a fresh `UIMessage[]`. Useful for
 * tests and for replaying a captured stream.
 */
export function raraEventsToUIMessages(events: PublicWebEvent[]): UIMessage[] {
  return events.reduce<UIMessage[]>((acc, ev) => applyRaraEvent(acc, ev), []);
}
