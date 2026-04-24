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

import type { AgentMessage } from '@mariozechner/pi-agent-core';
import type {
  AssistantMessage,
  TextContent,
  ThinkingContent,
  ToolCall,
  ToolResultMessage,
  UserMessage,
} from '@mariozechner/pi-ai';
import type { Attachment, UserMessageWithAttachments } from '@mariozechner/pi-web-ui';

import type { ChatMessageData } from '@/api/types';

/**
 * Detect whether a tool-result payload represents a failure. Mirrors the
 * backend's `is_failure_result` in `crates/app/src/tools/artifacts.rs`: a
 * bare string starting with `Error:` (pi-mono convention) or a JSON object
 * with an `error` key (kernel-serialized anyhow error).
 */
function isToolFailure(text: string): boolean {
  const trimmed = text.trimStart();
  if (trimmed.startsWith('Error:')) return true;
  try {
    const parsed = JSON.parse(trimmed);
    return (
      typeof parsed === 'object' && parsed !== null && !Array.isArray(parsed) && 'error' in parsed
    );
  } catch {
    return false;
  }
}

function mimeToFilename(mimeType: string, index: number): string {
  const ext = mimeType.split('/')[1] || 'bin';
  return `session-image-${index + 1}.${ext}`;
}

/** Zeroed usage — rara tracks usage server-side. */
const EMPTY_USAGE = {
  input: 0,
  output: 0,
  cacheRead: 0,
  cacheWrite: 0,
  totalTokens: 0,
  cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
};

/**
 * Parse assistant text into ThinkingContent + TextContent blocks.
 * `<think>reasoning</think>answer` → [{type:"thinking",...}, {type:"text",...}]
 */
function parseAssistantContent(raw: string): (TextContent | ThinkingContent)[] {
  const blocks: (TextContent | ThinkingContent)[] = [];
  const re = /<think>([\s\S]*?)<\/think>/g;
  let cursor = 0;
  let match: RegExpExecArray | null;

  while ((match = re.exec(raw)) !== null) {
    const before = raw.slice(cursor, match.index).trim();
    if (before) blocks.push({ type: 'text', text: before });
    const thinking = (match[1] ?? '').trim();
    if (thinking) blocks.push({ type: 'thinking', thinking });
    cursor = match.index + match[0].length;
  }

  const tail = raw.slice(cursor).trim();
  if (tail) blocks.push({ type: 'text', text: tail });

  return blocks;
}

/**
 * WeakMap from assistant `AgentMessage` object references to their
 * persisted `seq`. Populated by {@link toAgentMessages} and read by the
 * Lit assistant-message renderer when the user clicks one of the per-turn
 * trace buttons — the seq is then embedded on the dispatched CustomEvent
 * so the React layer can call the trace endpoint directly without any
 * timestamp-based lookup (which collided at second resolution).
 *
 * Keyed by object identity: the same references flow from
 * `toAgentMessages` → `agent.replaceMessages(...)` → pi-web-ui's
 * renderer, so the renderer sees the exact keys set here.
 *
 * Only the **final** assistant message of each turn is registered —
 * intermediate tool-call-only iterations are deliberately omitted so
 * trace buttons render exactly once per user-facing reply (#1672).
 */
export const assistantSeqByRef = new WeakMap<AgentMessage, number>();

/**
 * Module-level side-channel from `tool_call_id` → the persisted
 * {@link ToolResultMessage} for that call. Populated by
 * {@link toAgentMessages} instead of pushing standalone tool-result
 * bubbles into the message list — pi-web-ui's `<message-list>` assigns
 * one DOM row (and, via rara's CSS, one avatar) per message object,
 * so emitting a tool result as its own entry surfaced every result as
 * a bare avatar+bubble under the assistant's reply (#1718).
 *
 * The custom assistant renderer in `PiChat.tsx` reads this map to
 * build the `toolResultsById` lookup that pi-web-ui's
 * `<assistant-message>` needs to inline the result under its paired
 * `<tool-message>`. The same map is also consumed by
 * {@link messagesForArtifactReconstruction} so pi-web-ui's
 * `ArtifactsPanel.reconstructFromMessages` (which walks tool-result
 * messages to replay artifact operations) keeps working.
 *
 * Cleared at the start of every {@link toAgentMessages} call so a
 * session switch does not leak results from the previous session.
 */
export const toolResultByCallId = new Map<string, ToolResultMessage>();

/**
 * True when `msg` is the **first** assistant message of its turn inside
 * the provided live `AgentMessage[]` — i.e. the closest preceding
 * non-tool-result / non-assistant message is either absent (list head)
 * or a user/user-with-attachments message.
 *
 * The avatar + top-of-bubble chrome is painted only for this frame; every
 * subsequent assistant message in the same turn renders as a "continuation"
 * (no avatar, collapsed top margin) so the whole turn reads as one bubble
 * (#1727). Works uniformly for persisted history (emitted by
 * {@link toAgentMessages}) and live streaming (pi-agent-core pushes each
 * agentic-loop iteration as its own `AssistantMessage` into
 * `agent.state.messages`).
 *
 * `toolResult` frames — pi-agent-core appends them post-stream for live
 * turns — are transparent to the turn boundary: they neither open nor
 * close a turn. Only user messages do.
 *
 * `O(n)` on the message list; called at render time per assistant row.
 * For pi-web-ui's typical 200-message ceiling this is trivially cheap and
 * avoids maintaining a parallel cache that could desync from streaming
 * appends.
 */
export function isFirstAssistantOfTurn(msg: AgentMessage, all: readonly AgentMessage[]): boolean {
  // pi-agent-core emits per-iteration `AssistantMessage` frames that can
  // carry only an empty thinking-block; pi-web-ui's `:has()` avatar rules
  // do not match those, so they paint no avatar. Treating them as the
  // turn's first frame would anchor the avatar to an invisible row and
  // strip it from the real content. Skip them in both the self check and
  // the backward walk so the avatar lands on the first visible frame.
  if (msg.role === 'assistant' && !hasVisibleContent(msg)) return false;
  const idx = all.indexOf(msg);
  if (idx < 0) return true;
  for (let j = idx - 1; j >= 0; j--) {
    const prev = all[j];
    if (!prev) continue;
    if (prev.role === 'toolResult') continue;
    if (prev.role === 'assistant') {
      if (!hasVisibleContent(prev)) continue;
      return false;
    }
    // user / user-with-attachments / anything else is a turn boundary.
    return true;
  }
  return true;
}

function hasVisibleContent(msg: AssistantMessage): boolean {
  const content = msg.content;
  if (!Array.isArray(content)) return false;
  return content.some((part) => {
    if (part.type === 'text') return part.text.trim().length > 0;
    if (part.type === 'thinking') return part.thinking.trim().length > 0;
    if (part.type === 'toolCall') return true;
    return false;
  });
}

/**
 * For each turn (a contiguous run of non-user messages bounded by the
 * next user message or the end of the list), return the index in `msgs`
 * of the last `assistant`-role message in that turn. Indices not in the
 * returned set correspond to intermediate tool-call-only iterations
 * within a turn — they should remain visible as bubbles but carry no
 * trace buttons (see #1672).
 *
 * The trailing turn (no closing user message) is also included, so an
 * in-progress turn whose only frame so far is tool-call-only still gets
 * buttons rather than no buttons until the final reply lands.
 */
export function finalAssistantIndices(msgs: ChatMessageData[]): Set<number> {
  const finals = new Set<number>();
  let lastAssistantIdx: number | null = null;
  for (let i = 0; i < msgs.length; i++) {
    const m = msgs[i];
    if (!m) continue;
    const role = m.role;
    if (role === 'user') {
      if (lastAssistantIdx !== null) finals.add(lastAssistantIdx);
      lastAssistantIdx = null;
    } else if (role === 'assistant') {
      lastAssistantIdx = i;
    }
  }
  if (lastAssistantIdx !== null) finals.add(lastAssistantIdx);
  return finals;
}

/**
 * Convert rara `ChatMessageData` rows into pi-agent-core `AgentMessage`s
 * for display in the chat panel.
 *
 * Only the **final** assistant message of each turn is registered in
 * {@link assistantSeqByRef}; intermediate tool-call-only assistant
 * iterations are still emitted as bubbles but carry no trace buttons.
 * See {@link finalAssistantIndices} and #1672.
 */
export function toAgentMessages(msgs: ChatMessageData[]): AgentMessage[] {
  const result: AgentMessage[] = [];
  // Track the last assistant message so "tool" role messages can attach ToolCall items.
  let lastAssistant: AssistantMessage | null = null;
  const finals = finalAssistantIndices(msgs);
  // Reset the side-channel: subsequent loads must not see stale entries
  // from a previous session.
  toolResultByCallId.clear();

  for (let i = 0; i < msgs.length; i++) {
    const m = msgs[i];
    if (!m) continue;
    const ts = new Date(m.created_at).getTime();

    if (m.role === 'user') {
      lastAssistant = null;
      if (typeof m.content === 'string') {
        result.push({ role: 'user', content: m.content, timestamp: ts } as UserMessage);
      } else {
        const text = m.content
          .filter((b): b is { type: 'text'; text: string } => b.type === 'text')
          .map((b) => b.text)
          .join('\n');
        const attachments: Attachment[] = m.content.flatMap((b, index): Attachment[] => {
          if (b.type !== 'image_base64') return [];
          return [
            {
              id: `${m.seq}-image-${index}`,
              type: 'image',
              fileName: mimeToFilename(b.media_type, index),
              mimeType: b.media_type,
              size: Math.floor((b.data.length * 3) / 4),
              content: b.data,
              preview: b.data,
            },
          ];
        });

        if (attachments.length > 0) {
          result.push({
            role: 'user-with-attachments',
            content: text,
            attachments,
            timestamp: ts,
          } as UserMessageWithAttachments as AgentMessage);
        } else {
          result.push({ role: 'user', content: text, timestamp: ts } as UserMessage);
        }
      }
    } else if (m.role === 'assistant') {
      const raw =
        typeof m.content === 'string'
          ? m.content
          : m.content
              .filter((b): b is { type: 'text'; text: string } => b.type === 'text')
              .map((b) => b.text)
              .join('\n');
      const content: (TextContent | ThinkingContent | ToolCall)[] = parseAssistantContent(raw);
      // Surface persisted tool-call requests so pi-web-ui reducers (and the
      // artifacts panel's reconstructFromMessages) can see them.
      if (m.tool_calls && m.tool_calls.length > 0) {
        for (const tc of m.tool_calls) {
          const args =
            tc.arguments && typeof tc.arguments === 'object'
              ? (tc.arguments as Record<string, unknown>)
              : {};
          content.push({
            type: 'toolCall',
            id: tc.id,
            name: tc.name,
            arguments: args,
          });
        }
      }
      const assistant: AssistantMessage = {
        role: 'assistant',
        content,
        api: 'messages',
        provider: 'anthropic',
        model: 'unknown',
        usage: EMPTY_USAGE,
        stopReason: 'stop',
        timestamp: ts,
      };
      lastAssistant = assistant;
      // Only the final assistant of each turn carries trace buttons —
      // intermediate tool-call-only iterations stay anonymous (#1672).
      if (finals.has(i)) {
        assistantSeqByRef.set(assistant, m.seq);
      }
      result.push(assistant);
    } else if (m.role === 'tool') {
      // Tool call from the assistant — attach as ToolCall to the last AssistantMessage.
      if (lastAssistant && m.tool_call_id && m.tool_name) {
        let args: Record<string, unknown> = {};
        try {
          const raw = typeof m.content === 'string' ? m.content : JSON.stringify(m.content);
          args = JSON.parse(raw);
        } catch {
          /* use empty args */
        }
        const toolCall: ToolCall = {
          type: 'toolCall',
          id: m.tool_call_id,
          name: m.tool_name,
          arguments: args,
        };
        lastAssistant.content.push(toolCall);
      }
    } else if (m.role === 'tool_result') {
      // Tool result — DO NOT push as a standalone AgentMessage; that
      // made pi-web-ui's `<message-list>` render each result as its own
      // DOM row with its own avatar under rara's CSS, creating a
      // bare-bubble chain under the assistant reply (#1718). Instead
      // stash the result in `toolResultByCallId` for:
      //   1. `PiChat.tsx`'s custom assistant renderer, which builds the
      //      `toolResultsById` map that `<assistant-message>` uses to
      //      inline the result under its paired `<tool-message>`; and
      //   2. `ArtifactsPanel.reconstructFromMessages`, via
      //      {@link messagesForArtifactReconstruction} — artifacts
      //      replay walks tool-result messages to reconstruct state.
      //
      // Preserve the backend's failure markers so the artifacts panel
      // (which only replays successful ops) skips failed calls on
      // reload. The kernel serializes failures in two shapes: a bare
      // string starting with "Error:" (pi-mono convention) and JSON
      // objects with an `error` key (produced by the anyhow ->
      // ToolOutput path).
      if (m.tool_call_id && m.tool_name) {
        const text =
          typeof m.content === 'string'
            ? m.content
            : m.content
                .filter((b): b is { type: 'text'; text: string } => b.type === 'text')
                .map((b) => b.text)
                .join('\n');
        const toolResult: ToolResultMessage = {
          role: 'toolResult',
          toolCallId: m.tool_call_id,
          toolName: m.tool_name,
          content: text ? [{ type: 'text', text }] : [],
          isError: isToolFailure(text),
          timestamp: ts,
        };
        toolResultByCallId.set(m.tool_call_id, toolResult);
      }
    }
  }
  return result;
}

/**
 * Return the message list for display plus a parallel list augmented
 * with the suppressed tool-result bubbles, suitable for pi-web-ui's
 * `ArtifactsPanel.reconstructFromMessages` which pairs assistant
 * tool-calls with their `toolResult` responses to replay artifact
 * operations.
 *
 * The augmented list inserts each `ToolResultMessage` directly after
 * the assistant message that contains its matching `ToolCall`, so
 * reconstruction sees call/result pairs in the same relative order as
 * the backend persisted them. Results whose calls are not found (e.g.
 * a persisted tool result without its paired assistant tool-call on
 * this page) are appended at the end so nothing is silently dropped.
 *
 * Reads from {@link toolResultByCallId}, so this MUST be called after
 * {@link toAgentMessages} for the same message list — the side-channel
 * map is cleared on every `toAgentMessages` call.
 */
export function messagesForArtifactReconstruction(displayMessages: AgentMessage[]): AgentMessage[] {
  if (toolResultByCallId.size === 0) return displayMessages;
  const augmented: AgentMessage[] = [];
  const emitted = new Set<string>();
  for (const msg of displayMessages) {
    augmented.push(msg);
    if (msg.role !== 'assistant') continue;
    for (const part of msg.content) {
      if (part.type !== 'toolCall') continue;
      const tr = toolResultByCallId.get(part.id);
      if (tr && !emitted.has(part.id)) {
        augmented.push(tr as AgentMessage);
        emitted.add(part.id);
      }
    }
  }
  for (const [id, tr] of toolResultByCallId) {
    if (!emitted.has(id)) augmented.push(tr as AgentMessage);
  }
  return augmented;
}
