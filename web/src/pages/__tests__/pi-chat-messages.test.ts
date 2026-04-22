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

import type { AssistantMessage } from '@mariozechner/pi-ai';
import { describe, expect, it } from 'vitest';

import { assistantSeqByRef, finalAssistantIndices, toAgentMessages } from '../pi-chat-messages';

import type { ChatMessageData, ChatToolCallData } from '@/api/types';

const ISO = '2025-01-01T00:00:00Z';

function user(seq: number, text = 'hi'): ChatMessageData {
  return { seq, role: 'user', content: text, created_at: ISO };
}

function assistantText(seq: number, text = 'reply'): ChatMessageData {
  return { seq, role: 'assistant', content: text, created_at: ISO };
}

function assistantToolCall(seq: number, toolName = 'do_thing'): ChatMessageData {
  const call: ChatToolCallData = {
    id: `tc-${seq}`,
    name: toolName,
    arguments: {},
  };
  return {
    seq,
    role: 'assistant',
    content: '',
    tool_calls: [call],
    created_at: ISO,
  };
}

function toolResult(seq: number, callSeq: number, toolName = 'do_thing'): ChatMessageData {
  return {
    seq,
    role: 'tool_result',
    content: 'ok',
    tool_call_id: `tc-${callSeq}`,
    tool_name: toolName,
    created_at: ISO,
  };
}

function isRegistered(msg: AssistantMessage | undefined): boolean {
  if (!msg) throw new Error('expected an assistant message');
  return assistantSeqByRef.get(msg) !== undefined;
}

function expectAssistant(msg: AssistantMessage | undefined): AssistantMessage {
  if (!msg) throw new Error('expected an assistant message');
  return msg;
}

function assistants(out: ReturnType<typeof toAgentMessages>): AssistantMessage[] {
  return out.filter((m): m is AssistantMessage => m.role === 'assistant');
}

describe('finalAssistantIndices', () => {
  it('marks the only assistant of a single complete turn', () => {
    const msgs = [user(1), assistantText(2)];
    expect(finalAssistantIndices(msgs)).toEqual(new Set([1]));
  });

  it('marks only the last assistant when intermediate ones are tool-call-only', () => {
    const msgs = [user(1), assistantToolCall(2), assistantText(3)];
    expect(finalAssistantIndices(msgs)).toEqual(new Set([2]));
  });

  it('handles tool_result interleave between intermediate and final assistant', () => {
    const msgs = [user(1), assistantToolCall(2), toolResult(3, 2), assistantText(4)];
    expect(finalAssistantIndices(msgs)).toEqual(new Set([3]));
  });

  it('marks the final assistant of every turn across multiple turns', () => {
    const msgs = [user(1), assistantText(2), user(3), assistantToolCall(4), assistantText(5)];
    expect(finalAssistantIndices(msgs)).toEqual(new Set([1, 4]));
  });

  it('marks the trailing tool-call-only assistant on an open (incomplete) turn', () => {
    const msgs = [user(1), assistantToolCall(2)];
    expect(finalAssistantIndices(msgs)).toEqual(new Set([1]));
  });
});

describe('toAgentMessages — trace button registration (#1672)', () => {
  it('registers the lone assistant of a [user, assistant(text)] turn', () => {
    const out = toAgentMessages([user(1), assistantText(2)]);
    const [a] = assistants(out);
    expect(isRegistered(a)).toBe(true);
    expect(assistantSeqByRef.get(expectAssistant(a))).toBe(2);
  });

  it('registers only the final assistant in [user, assistant(tool_call), assistant(text)]', () => {
    const out = toAgentMessages([user(1), assistantToolCall(2), assistantText(3)]);
    const [intermediate, final] = assistants(out);
    expect(isRegistered(intermediate)).toBe(false);
    expect(isRegistered(final)).toBe(true);
    expect(assistantSeqByRef.get(expectAssistant(final))).toBe(3);
  });

  it('registers only the final assistant when a tool_result is interleaved', () => {
    const out = toAgentMessages([
      user(1),
      assistantToolCall(2),
      toolResult(3, 2),
      assistantText(4),
    ]);
    const [intermediate, final] = assistants(out);
    expect(isRegistered(intermediate)).toBe(false);
    expect(isRegistered(final)).toBe(true);
    expect(assistantSeqByRef.get(expectAssistant(final))).toBe(4);
  });

  it('registers the final assistant of every turn but not intermediate ones', () => {
    const out = toAgentMessages([
      user(1),
      assistantText(2),
      user(3),
      assistantToolCall(4),
      assistantText(5),
    ]);
    const [first, intermediate, last] = assistants(out);
    expect(isRegistered(first)).toBe(true);
    expect(assistantSeqByRef.get(expectAssistant(first))).toBe(2);
    expect(isRegistered(intermediate)).toBe(false);
    expect(isRegistered(last)).toBe(true);
    expect(assistantSeqByRef.get(expectAssistant(last))).toBe(5);
  });

  it('registers the trailing tool-call-only assistant on an open turn', () => {
    const out = toAgentMessages([user(1), assistantToolCall(2)]);
    const [a] = assistants(out);
    expect(isRegistered(a)).toBe(true);
    expect(assistantSeqByRef.get(expectAssistant(a))).toBe(2);
  });
});
