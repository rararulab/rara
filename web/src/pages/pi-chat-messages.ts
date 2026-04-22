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
      // Tool result — emit as a separate ToolResultMessage. Preserve the
      // backend's failure markers so ArtifactsPanel.reconstructFromMessages
      // (which only replays successful ops) skips failed calls on reload.
      // The kernel serializes failures in two shapes: a bare string starting
      // with "Error:" (pi-mono convention) and JSON objects with an `error`
      // key (produced by the anyhow -> ToolOutput path).
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
        result.push(toolResult as AgentMessage);
      }
    }
  }
  return result;
}
