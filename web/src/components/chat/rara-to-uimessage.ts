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
 * The chat shell (`PiChat`) consumes `UIMessage[]`, but rara's backend
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

/**
 * Metadata threaded onto persisted rara `UIMessage`s so the renderer can wire
 * trace / cascade triggers without re-deriving the row's `seq`.
 *
 * Live-streamed assistant frames don't know their seq (rara assigns it when
 * the turn lands in the kernel store); the value materialises only after
 * `historyToUIMessages` rebuilds the list from a REST refetch. Renderers
 * therefore gate trigger buttons on `metadata.seq !== undefined`.
 */
export interface RaraMessageMetadata {
  seq?: number;
}

/** UIMessage flavour used throughout the rara adapter + chat shell. */
export type RaraUIMessage = UIMessage<RaraMessageMetadata>;

/**
 * Stable id generator. UIMessage requires unique ids. We keep a single
 * `live-` prefix across both REST history (`live-history-...` would still
 * remount on refetch) and the streaming reducer — the goal is that ids
 * generated for a freshly-streamed turn don't visually collide with the ids
 * a subsequent history refetch will produce, so React's key-based
 * reconciliation does not remount the message.
 *
 * In practice rara's REST history numbers messages by `seq`, and live
 * streams don't know that seq up-front. We bridge by minting a synthetic
 * `live-${counter}` for the streaming tail; once the turn finalises and
 * history is refetched, the message is rebuilt with `msg-${seq}`. This
 * still causes one remount on history refetch — acceptable for now because
 * (a) the user has stopped typing, (b) the part contents are identical, and
 * (c) callers who want zero flicker can keep streaming-only state without
 * refetching. PR3+ replaces this with a stable backend-provided message id
 * threaded through the first delta.
 */
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
export function historyToUIMessages(history: ChatMessageData[]): RaraUIMessage[] {
  const messages: RaraUIMessage[] = [];
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
      messages.push({
        id: `msg-${row.seq}`,
        role: 'assistant',
        parts,
        metadata: { seq: row.seq },
      });
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
      if (!slot) {
        // History row references a tool call we never saw — log once for
        // observability so we notice if the backend stops emitting the
        // assistant frame that introduces the call.
        warnUnknown(`tool_result without matching tool_call: ${id}`);
        continue;
      }
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

/**
 * Return a fresh copy of `messages` where the tail assistant message has
 * been cloned (or appended if missing). The returned `msg` and its `parts`
 * array are safe to mutate — callers should mutate ONLY this clone, never
 * the original objects, so React reconciliation sees a new reference.
 */
function ensureAssistantTail(messages: RaraUIMessage[]): {
  next: RaraUIMessage[];
  msg: RaraUIMessage;
  index: number;
} {
  const last = messages[messages.length - 1];
  if (last && last.role === 'assistant') {
    const cloned: RaraUIMessage = { ...last, parts: [...last.parts] };
    const next = [...messages.slice(0, -1), cloned];
    return { next, msg: cloned, index: next.length - 1 };
  }
  const created: RaraUIMessage = {
    id: nextLiveId('assistant'),
    role: 'assistant',
    parts: [],
  };
  const next = [...messages, created];
  return { next, msg: created, index: next.length - 1 };
}

/** Append text onto the trailing text part of an assistant message, creating
 *  a new text part if the last one is something else (e.g. a tool call).
 *  MUTATES the passed `msg.parts` — caller must have already cloned it. */
function appendText(msg: RaraUIMessage, delta: string): void {
  const tail = msg.parts[msg.parts.length - 1] as RaraPart | undefined;
  if (tail && tail.type === 'text') {
    msg.parts[msg.parts.length - 1] = {
      ...tail,
      text: tail.text + delta,
      state: 'streaming',
    };
    return;
  }
  msg.parts.push({ type: 'text', text: delta, state: 'streaming' });
}

/** Append reasoning text similarly. MUTATES the passed `msg.parts`. */
function appendReasoning(msg: RaraUIMessage, delta: string): void {
  const tail = msg.parts[msg.parts.length - 1] as RaraPart | undefined;
  if (tail && tail.type === 'reasoning') {
    msg.parts[msg.parts.length - 1] = {
      ...tail,
      text: tail.text + delta,
      state: 'streaming',
    };
    return;
  }
  msg.parts.push({ type: 'reasoning', text: delta, state: 'streaming' });
}

/** Mark every still-streaming text/reasoning part on the assistant tail as
 *  done. Called when the run finishes so the renderer can drop streaming
 *  affordances (cursors, shimmer, etc). MUTATES the passed `msg.parts`. */
function markAssistantDone(msg: RaraUIMessage): void {
  msg.parts = msg.parts.map((part) =>
    part.type === 'text' || part.type === 'reasoning' ? { ...part, state: 'done' } : part,
  );
}

/** Locate a `dynamic-tool` part by its tool-call id across the message list,
 *  searching backwards because the active call is almost always on the tail. */
function findToolCall(
  messages: RaraUIMessage[],
  toolCallId: string,
): { msg: RaraUIMessage; part: DynamicToolUIPart; msgIndex: number; partIndex: number } | null {
  for (let i = messages.length - 1; i >= 0; i--) {
    const msg = messages[i];
    if (!msg || msg.role !== 'assistant') continue;
    for (let j = msg.parts.length - 1; j >= 0; j--) {
      const part = msg.parts[j];
      if (part && part.type === 'dynamic-tool' && part.toolCallId === toolCallId) {
        return { msg, part, msgIndex: i, partIndex: j };
      }
    }
  }
  return null;
}

/**
 * Pure reducer: apply one `PublicWebEvent` to a `UIMessage[]` and return a
 * new list with the touched message + its parts array cloned. Untouched
 * messages are referentially shared with the input so React memoisation on
 * unchanged messages still works, while reconciliation correctly invalidates
 * the message that actually changed.
 *
 * Variants we cannot map cleanly are logged (once) and skipped — never
 * thrown — so a stale frontend doesn't crash on a new backend variant.
 */
export function applyRaraEvent(messages: RaraUIMessage[], event: PublicWebEvent): RaraUIMessage[] {
  switch (event.type) {
    case '__stream_started':
    case '__stream_closed':
      // Lifecycle bookends. The caller may want these for connection state
      // but they do not change the message list.
      return messages;

    case 'text_delta': {
      const { next, msg } = ensureAssistantTail(messages);
      appendText(msg, event.text);
      return next;
    }

    case 'reasoning_delta': {
      const { next, msg } = ensureAssistantTail(messages);
      appendReasoning(msg, event.text);
      return next;
    }

    case 'tool_call_start': {
      const { next, msg } = ensureAssistantTail(messages);
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
      const found = findToolCall(messages, event.id);
      if (!found) return messages;
      const { part, msgIndex, partIndex } = found;
      const target = messages[msgIndex];
      if (!target) return messages;
      const newPart: DynamicToolUIPart = event.success
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
      const newParts = [...target.parts];
      newParts[partIndex] = newPart;
      const next = [...messages];
      next[msgIndex] = { ...target, parts: newParts };
      return next;
    }

    case 'message': {
      // One-shot complete message — rara collapses the whole turn into a
      // single frame. Treat it as a final text body on a fresh assistant.
      // The delta path above handles the streaming case; this branch only
      // fires when the backend never emits incremental text.
      const { next, msg } = ensureAssistantTail(messages);
      appendText(msg, event.content);
      markAssistantDone(msg);
      return next;
    }

    case 'done': {
      const tail = messages[messages.length - 1];
      if (!tail || tail.role !== 'assistant') return messages;
      const cloned: RaraUIMessage = { ...tail, parts: [...tail.parts] };
      markAssistantDone(cloned);
      return [...messages.slice(0, -1), cloned];
    }

    case 'error': {
      // Surface the error inline so the user sees what failed without
      // hunting in devtools. PR6 will polish the visual treatment.
      const { next, msg } = ensureAssistantTail(messages);
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
      return messages;

    default: {
      // Exhaustiveness guard: if a new variant lands the type narrows to
      // `never` and TS errors at compile time. At runtime we log once and
      // move on.
      const _exhaustive: never = event;
      void _exhaustive;
      warnUnknown(JSON.stringify(event));
      return messages;
    }
  }
}

/**
 * Fold a sequence of buffered events into a `UIMessage[]`.
 *
 * **Contract**: returns the SINGLE final-state array only. Do NOT slice or
 * snapshot intermediate accumulator states from inside `reduce` — successive
 * calls to {@link applyRaraEvent} share message-object references for
 * untouched entries, so an intermediate snapshot can have its own contents
 * change underneath you on a later iteration. For tests/replays that want
 * intermediate states, call {@link applyRaraEvent} yourself and take fresh
 * copies at each step.
 */
export function raraEventsToUIMessages(events: PublicWebEvent[]): RaraUIMessage[] {
  return events.reduce<RaraUIMessage[]>((acc, ev) => applyRaraEvent(acc, ev), []);
}
