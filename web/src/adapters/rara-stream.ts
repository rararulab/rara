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

import type { StreamFn } from "@mariozechner/pi-agent-core";
import type {
  AssistantMessage,
  AssistantMessageEvent,
  Context,
  ImageContent,
  Model,
  SimpleStreamOptions,
  TextContent,
  ThinkingContent,
  ToolCall,
  Usage,
} from "@mariozechner/pi-ai";
import { createAssistantMessageEventStream } from "@mariozechner/pi-ai";
import type { AssistantMessageEventStream } from "@mariozechner/pi-ai";

import { BASE_URL } from "@/api/client";

// ---------------------------------------------------------------------------
// WebEvent — frames received from the rara WebSocket chat API
// ---------------------------------------------------------------------------

/** Discriminated union of all WebSocket event types from the rara backend. */
type WebEvent =
  | { type: "text_delta"; text: string }
  | { type: "reasoning_delta"; text: string }
  | { type: "typing" }
  | {
      type: "tool_call_start";
      name: string;
      id: string;
      arguments: Record<string, unknown>;
    }
  | {
      type: "tool_call_end";
      id: string;
      result_preview: string;
      success: boolean;
      error: string | null;
    }
  | { type: "progress"; stage: string }
  | { type: "done" }
  | { type: "message"; content: string }
  | { type: "error"; message: string }
  | { type: "turn_rationale"; text: string }
  | {
      type: "turn_metrics";
      duration_ms: number;
      iterations: number;
      tool_calls: number;
      model: string;
    }
  | { type: "phase"; phase: string };

// ---------------------------------------------------------------------------
// Session key — provided via callback at stream time
// ---------------------------------------------------------------------------

/** Callback that returns the current session key for WebSocket connections. */
export type SessionKeyFn = () => string | undefined;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Build a zeroed Usage object — rara tracks usage server-side. */
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
): AssistantMessage {
  return {
    role: "assistant",
    content: [...content],
    api: model.api,
    provider: model.provider,
    model: model.id,
    usage: emptyUsage(),
    stopReason: "stop",
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
    const proto = loc.protocol === "https:" ? "wss:" : "ws:";
    base = `${proto}//${loc.host}`;
  } else {
    base = base.replace(/^http/, "ws");
  }

  // Strip trailing slash
  base = base.replace(/\/$/, "");

  return `${base}/api/v1/kernel/chat/ws?session_key=${encodeURIComponent(sessionKey)}&user_id=web_ryan`;
}

/**
 * Extract the latest user message content from a pi-ai Context.
 * Returns a plain string for text-only messages, or a JSON string
 * matching the backend InboundPayload format when images are present.
 */
function extractUserPayload(context: Context): string {
  for (let i = context.messages.length - 1; i >= 0; i--) {
    const msg = context.messages[i];
    if (msg.role === "user") {
      if (typeof msg.content === "string") return msg.content;

      // Check if there are any image blocks
      const hasImages = msg.content.some((c) => c.type === "image");

      if (!hasImages) {
        // Text-only — return plain string (backend parses as plain text)
        return msg.content
          .filter((c): c is TextContent => c.type === "text")
          .map((c) => c.text)
          .join("\n");
      }

      // Multimodal — build JSON payload matching backend InboundPayload.
      // Backend's parse_inbound_text_frame() tries JSON first, so this
      // will be deserialized as InboundPayload { content: MessageContent }.
      type RaraBlock =
        | { type: "text"; text: string }
        | { type: "image_base64"; media_type: string; data: string };
      const blocks: RaraBlock[] = msg.content.flatMap((c): RaraBlock[] => {
        if (c.type === "text") {
          return [{ type: "text", text: c.text }];
        }
        if (c.type === "image") {
          // pi-ai uses { mimeType, data }, rara uses { media_type, data }
          const img = c as ImageContent;
          if (img.mimeType && img.data) {
            return [{ type: "image_base64", media_type: img.mimeType, data: img.data }];
          }
        }
        return [];
      });
      return JSON.stringify({ content: blocks });
    }
  }
  return "";
}

// ---------------------------------------------------------------------------
// Stream function factory
// ---------------------------------------------------------------------------

/**
 * Create a StreamFn that bridges rara's WebSocket chat API to pi-ai events.
 * The `getSessionKey` callback is invoked at stream time to obtain the
 * current session key for the WebSocket connection.
 */
export function createRaraStreamFn(getSessionKey: SessionKeyFn): StreamFn {
  return (
    model: Model<any>,
    context: Context,
    _options?: SimpleStreamOptions,
  ): AssistantMessageEventStream => {
    const stream = createAssistantMessageEventStream();

    const sessionKey = getSessionKey();
    if (!sessionKey) {
      const errorMsg = buildPartial(model, [
        { type: "text", text: "No active session key set." },
      ]);
      errorMsg.stopReason = "error";
      errorMsg.errorMessage = "No active session key set.";
      stream.push({ type: "error", reason: "error", error: errorMsg });
      stream.end(errorMsg);
      return stream;
    }

    const userPayload = extractUserPayload(context);
    const wsUrl = buildWsUrl(sessionKey);

    // Accumulated content blocks for building partial messages
    const content: (TextContent | ThinkingContent | ToolCall)[] = [];
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
      if (last && last.type === "text") return last;
      const block: TextContent = { type: "text", text: "" };
      content.push(block);
      return block;
    }

    /** Find or create a thinking content block at the end of the content array. */
    function ensureThinkingBlock(): ThinkingContent {
      const last = content[content.length - 1];
      if (last && last.type === "thinking") return last;
      const block: ThinkingContent = { type: "thinking", thinking: "" };
      content.push(block);
      return block;
    }

    // Connect WebSocket asynchronously
    try {
      const ws = new WebSocket(wsUrl);

      ws.onopen = () => {
        // Emit start event
        safePush({ type: "start", partial: buildPartial(model, content) });
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
          case "text_delta": {
            const block = ensureTextBlock();
            const idx = content.indexOf(block);
            if (block.text === "") {
              // First delta for this block — emit text_start
              block.text = event.text;
              safePush({
                type: "text_start",
                contentIndex: idx,
                partial: buildPartial(model, content),
              });
            } else {
              block.text += event.text;
            }
            safePush({
              type: "text_delta",
              contentIndex: idx,
              delta: event.text,
              partial: buildPartial(model, content),
            });
            break;
          }

          case "reasoning_delta": {
            const block = ensureThinkingBlock();
            const idx = content.indexOf(block);
            if (block.thinking === "") {
              block.thinking = event.text;
              safePush({
                type: "thinking_start",
                contentIndex: idx,
                partial: buildPartial(model, content),
              });
            } else {
              block.thinking += event.text;
            }
            safePush({
              type: "thinking_delta",
              contentIndex: idx,
              delta: event.text,
              partial: buildPartial(model, content),
            });
            break;
          }

          case "tool_call_start": {
            const toolCall: ToolCall = {
              type: "toolCall",
              id: event.id,
              name: event.name,
              arguments: event.arguments,
            };
            content.push(toolCall);
            const idx = content.length - 1;
            safePush({
              type: "toolcall_start",
              contentIndex: idx,
              partial: buildPartial(model, content),
            });
            break;
          }

          case "tool_call_end": {
            const idx = content.findIndex(
              (c) => c.type === "toolCall" && c.id === event.id,
            );
            if (idx >= 0) {
              const toolCall = content[idx] as ToolCall;
              safePush({
                type: "toolcall_end",
                contentIndex: idx,
                toolCall,
                partial: buildPartial(model, content),
              });
            }
            break;
          }

          case "done": {
            // Close any open text/thinking blocks
            emitEndBlocks(model, content, safePush);

            const finalMsg = buildPartial(model, content);
            finalMsg.stopReason = "stop";
            safePush({ type: "done", reason: "stop", message: finalMsg });
            safeEnd(finalMsg);
            ws.close();
            break;
          }

          case "message": {
            // Complete message in a single frame — treat like text + done
            const block = ensureTextBlock();
            block.text += event.content;
            emitEndBlocks(model, content, safePush);

            const finalMsg = buildPartial(model, content);
            finalMsg.stopReason = "stop";
            safePush({ type: "done", reason: "stop", message: finalMsg });
            safeEnd(finalMsg);
            ws.close();
            break;
          }

          case "error": {
            const errorMsg = buildPartial(model, content);
            errorMsg.stopReason = "error";
            errorMsg.errorMessage = event.message;
            safePush({ type: "error", reason: "error", error: errorMsg });
            safeEnd(errorMsg);
            ws.close();
            break;
          }

          // Informational events — ignored for now
          case "typing":
          case "progress":
          case "turn_rationale":
          case "turn_metrics":
          case "phase":
            break;
        }
      };

      ws.onerror = () => {
        const errorMsg = buildPartial(model, content);
        errorMsg.stopReason = "error";
        errorMsg.errorMessage = "WebSocket connection error";
        safePush({ type: "error", reason: "error", error: errorMsg });
        safeEnd(errorMsg);
      };

      ws.onclose = () => {
        // Ensure stream is ended if WS closes unexpectedly
        if (!streamEnded) {
          const finalMsg = buildPartial(model, content);
          finalMsg.stopReason = content.length > 0 ? "stop" : "error";
          if (content.length > 0) {
            safePush({ type: "done", reason: "stop", message: finalMsg });
          } else {
            finalMsg.errorMessage = "WebSocket closed unexpectedly";
            safePush({ type: "error", reason: "error", error: finalMsg });
          }
          safeEnd(finalMsg);
        }
      };
    } catch (err) {
      const errorMsg = buildPartial(model, content);
      errorMsg.stopReason = "error";
      errorMsg.errorMessage =
        err instanceof Error ? err.message : "Failed to connect";
      stream.push({ type: "error", reason: "error", error: errorMsg });
      stream.end(errorMsg);
    }

    return stream;
  };
}

/**
 * Emit text_end / thinking_end events for any open content blocks.
 * Called before the final done/message event.
 */
function emitEndBlocks(
  model: Model<any>,
  content: (TextContent | ThinkingContent | ToolCall)[],
  safePush: (event: AssistantMessageEvent) => void,
): void {
  for (let i = 0; i < content.length; i++) {
    const block = content[i];
    if (block.type === "text" && block.text) {
      safePush({
        type: "text_end",
        contentIndex: i,
        content: block.text,
        partial: buildPartial(model, content),
      });
    } else if (block.type === "thinking" && block.thinking) {
      safePush({
        type: "thinking_end",
        contentIndex: i,
        content: block.thinking,
        partial: buildPartial(model, content),
      });
    }
  }
}
