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

import type { AgentTool, AgentToolResult, StreamFn } from '@mariozechner/pi-agent-core';
import { calculateCost, createAssistantMessageEventStream } from '@mariozechner/pi-ai';
import type {
  AssistantMessage,
  AssistantMessageEvent,
  Context,
  Model,
  SimpleStreamOptions,
  TextContent,
  ThinkingContent,
  ToolCall,
  Usage,
} from '@mariozechner/pi-ai';
import type { AssistantMessageEventStream } from '@mariozechner/pi-ai';
import type { Attachment } from '@mariozechner/pi-web-ui';
import { Type } from '@sinclair/typebox';

import { BASE_URL } from '@/api/client';

// ---------------------------------------------------------------------------
// WebEvent — frames received from the rara WebSocket chat API
// ---------------------------------------------------------------------------

/** Discriminated union of all WebSocket event types from the rara backend. */
type WebEvent =
  | { type: 'text_delta'; text: string }
  | { type: 'reasoning_delta'; text: string }
  | { type: 'typing' }
  | {
      type: 'tool_call_start';
      name: string;
      id: string;
      arguments: Record<string, unknown>;
    }
  | {
      type: 'tool_call_end';
      id: string;
      result_preview: string;
      success: boolean;
      error: string | null;
    }
  | { type: 'progress'; stage: string }
  | { type: 'done' }
  | { type: 'message'; content: string }
  | { type: 'error'; message: string }
  | { type: 'turn_rationale'; text: string }
  | {
      type: 'turn_metrics';
      duration_ms: number;
      iterations: number;
      tool_calls: number;
      model: string;
    }
  | {
      type: 'usage';
      input: number;
      output: number;
      cache_read: number;
      cache_write: number;
      total_tokens: number;
      cost: number;
      model: string;
    }
  | { type: 'phase'; phase: string };

// ---------------------------------------------------------------------------
// Session key — provided via callback at stream time
// ---------------------------------------------------------------------------

/** Callback that returns the current session key for WebSocket connections. */
export type SessionKeyFn = () => string | undefined;

/**
 * Callback that returns raw attachments associated with the pending user
 * turn. Documents (PDF/DOCX/XLSX/PPTX) are serialized as `file_base64`
 * blocks so the backend can forward the raw bytes to tools / multimodal
 * models while still receiving the client-side extracted text via the
 * pi-mono attachment pipeline.
 */
export type PendingAttachmentsFn = () => Attachment[] | undefined;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/**
 * Build a zeroed Usage object — used as the initial value before the
 * backend streams its final `usage` event. Cost is filled in from the
 * session's model pricing table via {@link calculateCost} once real
 * token counts arrive.
 */
function emptyUsage(): Usage {
  return {
    input: 0,
    output: 0,
    cacheRead: 0,
    cacheWrite: 0,
    totalTokens: 0,
    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
  };
}

/** Build a partial AssistantMessage snapshot from accumulated state. */
function buildPartial(
  model: Model<any>,
  content: (TextContent | ThinkingContent | ToolCall)[],
  usage: Usage,
): AssistantMessage {
  return {
    role: 'assistant',
    content: [...content],
    api: model.api,
    provider: model.provider,
    model: model.id,
    usage,
    stopReason: 'stop',
    timestamp: Date.now(),
  };
}

/**
 * Derive the WebSocket URL from the configured API base URL.
 * Converts http(s) to ws(s) and appends the chat WS path.
 */
export function buildWsUrl(sessionKey: string): string {
  let base = BASE_URL;

  // When BASE_URL is empty, derive from current page location
  if (!base) {
    const loc = window.location;
    const proto = loc.protocol === 'https:' ? 'wss:' : 'ws:';
    base = `${proto}//${loc.host}`;
  } else {
    base = base.replace(/^http/, 'ws');
  }

  // Strip trailing slash
  base = base.replace(/\/$/, '');

  return `${base}/api/v1/kernel/chat/ws?session_key=${encodeURIComponent(sessionKey)}&user_id=web_ryan`;
}

/**
 * Wire-format block sent to the backend `InboundPayload.content` field.
 * Mirrors rara's `ChatContentBlock` (crates/kernel/src/channel/types.rs).
 */
type RaraBlock =
  | { type: 'text'; text: string }
  | { type: 'image_base64'; media_type: string; data: string }
  | {
      type: 'file_base64';
      media_type: string;
      data: string;
      filename?: string;
    };

/**
 * Extract the latest user message content from a pi-ai Context and
 * augment it with non-image document attachments as `file_base64` blocks.
 *
 * Returns a plain string for text-only messages with no attachments, or a
 * JSON string matching the backend `InboundPayload` when images or raw
 * document bytes need to be forwarded.
 */
function extractUserPayload(context: Context, attachments: Attachment[]): string {
  for (let i = context.messages.length - 1; i >= 0; i--) {
    const msg = context.messages[i];
    if (msg && msg.role === 'user') {
      const hasImages =
        typeof msg.content !== 'string' && msg.content.some((c) => c.type === 'image');
      const documentAttachments = attachments.filter((a) => a.type === 'document');

      if (typeof msg.content === 'string') {
        if (documentAttachments.length === 0) return msg.content;
        const blocks: RaraBlock[] = [{ type: 'text', text: msg.content }];
        for (const doc of documentAttachments) {
          blocks.push({
            type: 'file_base64',
            media_type: doc.mimeType,
            data: doc.content,
            filename: doc.fileName,
          });
        }
        return JSON.stringify({ content: blocks });
      }

      if (!hasImages && documentAttachments.length === 0) {
        // Text-only — return plain string (backend parses as plain text)
        return msg.content
          .filter((c): c is TextContent => c.type === 'text')
          .map((c) => c.text)
          .join('\n');
      }

      // Multimodal — build JSON payload matching backend InboundPayload.
      // Backend's parse_inbound_text_frame() tries JSON first, so this
      // will be deserialized as InboundPayload { content: MessageContent }.
      const blocks: RaraBlock[] = msg.content.flatMap((c): RaraBlock[] => {
        if (c.type === 'text') {
          return [{ type: 'text', text: c.text }];
        }
        if (c.type === 'image') {
          // pi-ai uses { mimeType, data }, rara uses { media_type, data }
          const img = c;
          if (img.mimeType && img.data) {
            return [{ type: 'image_base64', media_type: img.mimeType, data: img.data }];
          }
        }
        return [];
      });
      for (const doc of documentAttachments) {
        blocks.push({
          type: 'file_base64',
          media_type: doc.mimeType,
          data: doc.content,
          filename: doc.fileName,
        });
      }
      return JSON.stringify({ content: blocks });
    }
  }
  return '';
}

// ---------------------------------------------------------------------------
// Kernel-authoritative tool results
//
// The rara backend runs its own agent loop in Rust — when the LLM emits
// tool calls we receive `tool_call_start` followed by `tool_call_end`
// carrying the real result. pi-agent-core, however, has its own frontend
// loop (`agent-loop.js`) that inspects the final assistant message for
// `toolCall` blocks and, finding none of our tools in `context.tools`,
// synthesises `Tool ${name} not found` error results that stomp the real
// kernel output (see issue #1601).
//
// The fix is to hand pi-agent-core `AgentTool` entries whose `execute`
// function *awaits* the kernel's already-dispatched result. Each
// `tool_call_start` seeds a pending entry; the matching `tool_call_end`
// resolves it; the pi-agent-core loop then treats our relay as the
// authoritative executor and threads the real result back into the
// message list under the correct `toolCallId`.
// ---------------------------------------------------------------------------

/** Pending result slot awaiting the matching `tool_call_end` frame. */
interface PendingToolResult {
  promise: Promise<AgentToolResult<unknown>>;
  resolve: (result: AgentToolResult<unknown>) => void;
  reject: (error: Error) => void;
  /** Cached after resolution so late `execute()` calls still get a value. */
  resolved: AgentToolResult<unknown> | null;
}

/** Shared promise schema exposed by pi-ai's TypeBox parameters. */
const OPAQUE_PARAMETERS = Type.Record(Type.String(), Type.Unknown());

/**
 * Build an `AgentTool` shim whose `execute` resolves from the kernel's
 * `tool_call_end` payload tracked in {@link pending}. The shim is
 * installed into `context.tools` on demand so pi-agent-core's loop finds
 * it by name and never falls back to the `Tool ${name} not found` path.
 */
function makeRelayTool(name: string, pending: Map<string, PendingToolResult>): AgentTool {
  return {
    name,
    label: name,
    description: `Kernel-executed tool ${name}. Results are relayed by rara-stream.`,
    parameters: OPAQUE_PARAMETERS,
    execute: async (toolCallId) => {
      const slot = pending.get(toolCallId);
      if (!slot) {
        // Defensive: the `tool_call_start` frame should always arrive
        // before pi-agent-core reaches the execute step, but if the
        // stream ends abnormally we surface a clear diagnostic instead
        // of hanging the loop forever.
        throw new Error(`No kernel result registered for tool call ${toolCallId} (${name})`);
      }
      return slot.promise;
    },
  };
}

// ---------------------------------------------------------------------------
// Stream function factory
// ---------------------------------------------------------------------------

/**
 * Create a StreamFn that bridges rara's WebSocket chat API to pi-ai events.
 * The `getSessionKey` callback is invoked at stream time to obtain the
 * current session key for the WebSocket connection. The optional
 * `getPendingAttachments` callback surfaces raw attachments (in particular
 * document base64 bytes) so they can be forwarded to the backend as
 * `file_base64` blocks alongside the text-extracted content that pi-mono
 * already inserts into the pi-ai Context.
 */
export function createRaraStreamFn(
  getSessionKey: SessionKeyFn,
  getPendingAttachments?: PendingAttachmentsFn,
): StreamFn {
  return (
    model: Model<any>,
    context: Context,
    _options?: SimpleStreamOptions,
  ): AssistantMessageEventStream => {
    const stream = createAssistantMessageEventStream();

    const sessionKey = getSessionKey();
    if (!sessionKey) {
      const errorMsg = buildPartial(
        model,
        [{ type: 'text', text: 'No active session key set.' }],
        emptyUsage(),
      );
      errorMsg.stopReason = 'error';
      errorMsg.errorMessage = 'No active session key set.';
      stream.push({ type: 'error', reason: 'error', error: errorMsg });
      stream.end(errorMsg);
      return stream;
    }

    const userPayload = extractUserPayload(context, getPendingAttachments?.() ?? []);
    const wsUrl = buildWsUrl(sessionKey);

    // Accumulated content blocks for building partial messages
    const content: (TextContent | ThinkingContent | ToolCall)[] = [];
    // Per-turn registry of kernel-authoritative tool results keyed by
    // `toolCallId`. The companion `AgentTool` shims installed into
    // `context.tools` await these promises so pi-agent-core's loop
    // threads the real result through without the "Tool X not found"
    // fallback (see #1601).
    const pendingToolResults = new Map<string, PendingToolResult>();
    // Deduplicate shim installation — one `AgentTool` per distinct name.
    const installedTools = new Set<string>();
    // Ensure `context.tools` is an array we can mutate in place so the
    // shims are visible when pi-agent-core's loop reads `currentContext.tools`
    // after this stream ends.
    if (!context.tools) context.tools = [];
    const contextTools = context.tools;
    const installedNamesFromContext = new Set(contextTools.map((t) => t.name));
    // Running usage — starts empty, replaced when the backend emits its
    // final `usage` event. Cost is computed against the session model's
    // pricing table so per-session model overrides are honoured.
    let currentUsage: Usage = emptyUsage();
    let streamEnded = false;

    /** Push an event to the stream, guarding against double-end. */
    function safePush(event: AssistantMessageEvent): void {
      if (!streamEnded) stream.push(event);
    }

    /** End the stream with a final message, guarding against double-end. */
    function safeEnd(msg: AssistantMessage): void {
      if (streamEnded) return;
      streamEnded = true;
      stream.end(msg);
    }

    /** Find or create a text content block at the end of the content array. */
    function ensureTextBlock(): TextContent {
      const last = content[content.length - 1];
      if (last && last.type === 'text') return last;
      const block: TextContent = { type: 'text', text: '' };
      content.push(block);
      return block;
    }

    /** Find or create a thinking content block at the end of the content array. */
    function ensureThinkingBlock(): ThinkingContent {
      const last = content[content.length - 1];
      if (last && last.type === 'thinking') return last;
      const block: ThinkingContent = { type: 'thinking', thinking: '' };
      content.push(block);
      return block;
    }

    // Connect WebSocket asynchronously
    try {
      const ws = new WebSocket(wsUrl);

      ws.onopen = () => {
        // Emit start event
        safePush({ type: 'start', partial: buildPartial(model, content, currentUsage) });
        // Send user message
        ws.send(userPayload);
      };

      ws.onmessage = (ev: MessageEvent) => {
        let event: WebEvent;
        try {
          event = JSON.parse(ev.data as string) as WebEvent;
        } catch {
          return; // Ignore non-JSON frames
        }

        switch (event.type) {
          case 'text_delta': {
            const block = ensureTextBlock();
            const idx = content.indexOf(block);
            if (block.text === '') {
              // First delta for this block — emit text_start
              block.text = event.text;
              safePush({
                type: 'text_start',
                contentIndex: idx,
                partial: buildPartial(model, content, currentUsage),
              });
            } else {
              block.text += event.text;
            }
            safePush({
              type: 'text_delta',
              contentIndex: idx,
              delta: event.text,
              partial: buildPartial(model, content, currentUsage),
            });
            break;
          }

          case 'reasoning_delta': {
            const block = ensureThinkingBlock();
            const idx = content.indexOf(block);
            if (block.thinking === '') {
              block.thinking = event.text;
              safePush({
                type: 'thinking_start',
                contentIndex: idx,
                partial: buildPartial(model, content, currentUsage),
              });
            } else {
              block.thinking += event.text;
            }
            safePush({
              type: 'thinking_delta',
              contentIndex: idx,
              delta: event.text,
              partial: buildPartial(model, content, currentUsage),
            });
            break;
          }

          case 'tool_call_start': {
            const toolCall: ToolCall = {
              type: 'toolCall',
              id: event.id,
              name: event.name,
              arguments: event.arguments,
            };
            content.push(toolCall);
            const idx = content.length - 1;
            // Register the pending result slot BEFORE pi-agent-core's
            // post-stream loop looks up executors. Also install a shim
            // entry in `context.tools` so the lookup resolves.
            let resolveFn: (value: AgentToolResult<unknown>) => void = () => {};
            let rejectFn: (err: Error) => void = () => {};
            const promise = new Promise<AgentToolResult<unknown>>((res, rej) => {
              resolveFn = res;
              rejectFn = rej;
            });
            pendingToolResults.set(event.id, {
              promise,
              resolve: resolveFn,
              reject: rejectFn,
              resolved: null,
            });
            if (!installedTools.has(event.name) && !installedNamesFromContext.has(event.name)) {
              contextTools.push(makeRelayTool(event.name, pendingToolResults));
              installedTools.add(event.name);
            }
            safePush({
              type: 'toolcall_start',
              contentIndex: idx,
              partial: buildPartial(model, content, currentUsage),
            });
            break;
          }

          case 'tool_call_end': {
            const idx = content.findIndex((c) => c.type === 'toolCall' && c.id === event.id);
            if (idx >= 0) {
              const toolCall = content[idx] as ToolCall;
              safePush({
                type: 'toolcall_end',
                contentIndex: idx,
                toolCall,
                partial: buildPartial(model, content, currentUsage),
              });
            }
            // Resolve the pending slot with the kernel's authoritative
            // result so the relay `AgentTool.execute` returns the real
            // text content (or a structured error) to pi-agent-core's
            // loop. `result_preview` is the backend-truncated form of
            // the tool output — good enough for UI rendering, which is
            // the only consumer (pi-agent-core feeds it back into the
            // message list as a `toolResult` message; the LLM never
            // sees this client-side copy, the server re-injects the
            // untruncated version on the next turn via tape memory).
            const slot = pendingToolResults.get(event.id);
            if (slot) {
              const text = event.error ?? event.result_preview;
              const result: AgentToolResult<unknown> = {
                content: [{ type: 'text', text }],
                details: {},
              };
              slot.resolved = result;
              slot.resolve(result);
            }
            break;
          }

          case 'done': {
            // Close any open text/thinking blocks
            emitEndBlocks(model, content, currentUsage, safePush);

            const finalMsg = buildPartial(model, content, currentUsage);
            finalMsg.stopReason = 'stop';
            safePush({ type: 'done', reason: 'stop', message: finalMsg });
            safeEnd(finalMsg);
            ws.close();
            break;
          }

          case 'message': {
            // Complete message in a single frame — treat like text + done
            const block = ensureTextBlock();
            block.text += event.content;
            emitEndBlocks(model, content, currentUsage, safePush);

            const finalMsg = buildPartial(model, content, currentUsage);
            finalMsg.stopReason = 'stop';
            safePush({ type: 'done', reason: 'stop', message: finalMsg });
            safeEnd(finalMsg);
            ws.close();
            break;
          }

          case 'error': {
            const errorMsg = buildPartial(model, content, currentUsage);
            errorMsg.stopReason = 'error';
            errorMsg.errorMessage = event.message;
            safePush({ type: 'error', reason: 'error', error: errorMsg });
            safeEnd(errorMsg);
            ws.close();
            break;
          }

          case 'usage': {
            // Backend reports raw token counts; cost comes from pi-ai's
            // pricing table for the session's model so per-session
            // overrides are honoured without duplicating pricing in Rust.
            const next: Usage = {
              input: event.input,
              output: event.output,
              cacheRead: event.cache_read,
              cacheWrite: event.cache_write,
              totalTokens: event.total_tokens,
              cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
            };
            next.cost = calculateCost(model, next);
            currentUsage = next;
            break;
          }

          // Informational events — ignored for now
          case 'typing':
          case 'progress':
          case 'turn_rationale':
          case 'turn_metrics':
          case 'phase':
            break;
        }
      };

      ws.onerror = () => {
        const errorMsg = buildPartial(model, content, currentUsage);
        errorMsg.stopReason = 'error';
        errorMsg.errorMessage = 'WebSocket connection error';
        safePush({ type: 'error', reason: 'error', error: errorMsg });
        safeEnd(errorMsg);
        rejectPendingToolResults(pendingToolResults, 'WebSocket connection error');
      };

      ws.onclose = () => {
        // Ensure stream is ended if WS closes unexpectedly
        if (!streamEnded) {
          const finalMsg = buildPartial(model, content, currentUsage);
          finalMsg.stopReason = content.length > 0 ? 'stop' : 'error';
          if (content.length > 0) {
            safePush({ type: 'done', reason: 'stop', message: finalMsg });
          } else {
            finalMsg.errorMessage = 'WebSocket closed unexpectedly';
            safePush({ type: 'error', reason: 'error', error: finalMsg });
          }
          safeEnd(finalMsg);
        }
        rejectPendingToolResults(pendingToolResults, 'WebSocket closed before tool result');
      };
    } catch (err) {
      const errorMsg = buildPartial(model, content, currentUsage);
      errorMsg.stopReason = 'error';
      errorMsg.errorMessage = err instanceof Error ? err.message : 'Failed to connect';
      stream.push({ type: 'error', reason: 'error', error: errorMsg });
      stream.end(errorMsg);
    }

    return stream;
  };
}

/**
 * Fail any tool-result promises the kernel never finished. Called from
 * `ws.onerror` / `ws.onclose` so pi-agent-core's loop sees a concrete
 * rejection rather than hanging on an abandoned `tool_call_start`.
 */
function rejectPendingToolResults(pending: Map<string, PendingToolResult>, reason: string): void {
  for (const slot of pending.values()) {
    if (slot.resolved === null) slot.reject(new Error(reason));
  }
}

/**
 * Emit text_end / thinking_end events for any open content blocks.
 * Called before the final done/message event.
 */
function emitEndBlocks(
  model: Model<any>,
  content: (TextContent | ThinkingContent | ToolCall)[],
  usage: Usage,
  safePush: (event: AssistantMessageEvent) => void,
): void {
  for (let i = 0; i < content.length; i++) {
    const block = content[i];
    if (!block) continue;
    if (block.type === 'text' && block.text) {
      safePush({
        type: 'text_end',
        contentIndex: i,
        content: block.text,
        partial: buildPartial(model, content, usage),
      });
    } else if (block.type === 'thinking' && block.thinking) {
      safePush({
        type: 'thinking_end',
        contentIndex: i,
        content: block.thinking,
        partial: buildPartial(model, content, usage),
      });
    }
  }
}
