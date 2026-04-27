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

import type { Api, AssistantMessage, Model } from '@mariozechner/pi-ai';
import { describe, expect, it, vi } from 'vitest';

import { RaraAgent } from '@/agent/rara-agent';
import {
  type FrameHandler,
  type LifecycleEvent,
  type LifecycleHandler,
  SessionWsClient,
  type WebFrame,
} from '@/agent/session-ws-client';
import type { RaraAgentEvent } from '@/agent/types';

// ---------------------------------------------------------------------------
// FakeSessionWsClient — drop-in SessionWsClient that lets tests push frames
// without a real WebSocket. Implements the same surface RaraAgent calls.
// ---------------------------------------------------------------------------

class FakeSessionWsClient {
  frameHandlers: FrameHandler[] = [];
  lifecycleHandlers: LifecycleHandler[] = [];
  connected = false;
  sentPrompts: unknown[] = [];
  sentAborts = 0;

  onFrame(h: FrameHandler): () => void {
    this.frameHandlers.push(h);
    return () => {
      this.frameHandlers = this.frameHandlers.filter((f) => f !== h);
    };
  }

  onLifecycle(h: LifecycleHandler): () => void {
    this.lifecycleHandlers.push(h);
    return () => {
      this.lifecycleHandlers = this.lifecycleHandlers.filter((f) => f !== h);
    };
  }

  connect(): void {
    this.connected = true;
    // Auto-emit the `hello` proof-of-life frame immediately so prompt()
    // sees an "open" socket. Real clients reset the retry budget here.
    for (const h of [...this.lifecycleHandlers]) h({ type: 'connected' });
    this.fire({ type: 'hello' });
  }

  disconnect(): void {
    this.connected = false;
  }

  prompt(content: unknown): boolean {
    if (!this.connected) return false;
    this.sentPrompts.push(content);
    return true;
  }

  abort(): boolean {
    if (!this.connected) return false;
    this.sentAborts += 1;
    return true;
  }

  fire(frame: WebFrame): void {
    for (const h of [...this.frameHandlers]) h(frame);
  }

  fireLifecycle(event: LifecycleEvent): void {
    for (const h of [...this.lifecycleHandlers]) h(event);
  }
}

const TEST_MODEL: Model<Api> = {
  id: 'test-model',
  name: 'Test',
  api: 'anthropic-messages',
  provider: 'anthropic',
  baseUrl: 'http://localhost',
  reasoning: false,
  input: ['text'],
  cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
  contextWindow: 100_000,
  maxTokens: 4_096,
};

function makeAgent(): { agent: RaraAgent; client: FakeSessionWsClient } {
  const client = new FakeSessionWsClient();
  const agent = new RaraAgent({
    sessionId: 'sess-1',
    model: TEST_MODEL,
    clientFactory: () => client as unknown as SessionWsClient,
  });
  return { agent, client };
}

describe('RaraAgent', () => {
  it('subscribe/unsubscribe', () => {
    const { agent } = makeAgent();
    const events: RaraAgentEvent[] = [];
    const off = agent.subscribe((e) => events.push(e));
    agent.appendMessage({ role: 'user', content: 'x', timestamp: 0 });
    expect(events).toHaveLength(2); // message_start + message_end
    off();
    agent.appendMessage({ role: 'user', content: 'y', timestamp: 0 });
    expect(events).toHaveLength(2); // unchanged
  });

  it('prompt() drives a clean text-only turn', async () => {
    const { agent, client } = makeAgent();
    const events: RaraAgentEvent[] = [];
    agent.subscribe((e) => events.push(e));

    await agent.prompt('hello');
    expect(client.sentPrompts).toEqual(['hello']);

    client.fire({ type: 'text_delta', text: 'hi' });
    client.fire({ type: 'text_delta', text: ' there' });
    client.fire({ type: 'done' });

    const types = events.map((e) => e.type);
    expect(types).toEqual([
      'agent_start',
      'turn_start',
      'message_start', // user
      'message_end', // user
      'message_start', // assistant partial
      'message_update',
      'message_update',
      'message_end',
      'turn_end',
      'agent_end',
    ]);

    expect(agent.state.isStreaming).toBe(false);
    expect(agent.state.streamMessage).toBeNull();
    expect(agent.state.messages).toHaveLength(2);
    const final = agent.state.messages[1] as AssistantMessage;
    expect(final.role).toBe('assistant');
    expect(final.content).toEqual([{ type: 'text', text: 'hi there' }]);
    expect(final.stopReason).toBe('stop');
  });

  it('throws if prompt() called while streaming', async () => {
    const { agent } = makeAgent();
    await agent.prompt('first');
    await expect(agent.prompt('second')).rejects.toThrow(/already processing/);
  });

  it('abort() sends abort frame and terminates the turn locally', async () => {
    const { agent, client } = makeAgent();
    const events: RaraAgentEvent[] = [];
    agent.subscribe((e) => events.push(e));

    await agent.prompt('hello');
    client.fire({ type: 'text_delta', text: 'partial' });
    agent.abort();

    expect(client.sentAborts).toBe(1);
    const last = events.at(-1)!;
    expect(last.type).toBe('agent_end');
    expect(agent.state.isStreaming).toBe(false);
    const errMsg = agent.state.messages.at(-1) as AssistantMessage;
    expect(errMsg.stopReason).toBe('aborted');
    expect(errMsg.errorMessage).toBe('Aborted by user');
  });

  it('handles tool_call_start / attachment / tool_call_end ordering', async () => {
    const { agent, client } = makeAgent();
    const events: RaraAgentEvent[] = [];
    agent.subscribe((e) => events.push(e));

    await agent.prompt('use a tool');
    client.fire({
      type: 'tool_call_start',
      id: 't1',
      name: 'search',
      arguments: { q: 'rust' },
    });
    expect(agent.state.pendingToolCalls.has('t1')).toBe(true);
    client.fire({
      type: 'attachment',
      tool_call_id: 't1',
      mime_type: 'image/png',
      filename: null,
      data_base64: 'AAAA',
    });
    client.fire({
      type: 'tool_call_end',
      id: 't1',
      result_preview: 'ok',
      success: true,
      error: null,
    });
    expect(agent.state.pendingToolCalls.has('t1')).toBe(false);
    client.fire({ type: 'done' });

    const turnEnd = events.find((e) => e.type === 'turn_end');
    expect(turnEnd).toBeDefined();
    if (turnEnd?.type !== 'turn_end') throw new Error('unreachable');
    expect(turnEnd.toolResults).toHaveLength(1);
    expect(turnEnd.toolResults[0]!.toolCallId).toBe('t1');
    expect(turnEnd.toolResults[0]!.toolName).toBe('search');
    expect(turnEnd.toolResults[0]!.content).toEqual([
      { type: 'text', text: 'ok' },
      { type: 'image', data: 'AAAA', mimeType: 'image/png' },
    ]);
  });

  it('multi-turn within one connection: state resets between prompts', async () => {
    const { agent, client } = makeAgent();
    const events: RaraAgentEvent[] = [];
    agent.subscribe((e) => events.push(e));

    await agent.prompt('first');
    client.fire({ type: 'text_delta', text: 'one' });
    client.fire({ type: 'done' });
    expect(agent.state.isStreaming).toBe(false);
    expect(agent.state.messages).toHaveLength(2);

    events.length = 0;
    await agent.prompt('second');
    client.fire({ type: 'text_delta', text: 'two' });
    client.fire({ type: 'done' });

    expect(agent.state.messages).toHaveLength(4);
    expect(events.filter((e) => e.type === 'agent_start')).toHaveLength(1);
    expect(events.filter((e) => e.type === 'agent_end')).toHaveLength(1);
  });

  it('error frame terminates the turn with stopReason=error', async () => {
    const { agent, client } = makeAgent();
    const events: RaraAgentEvent[] = [];
    agent.subscribe((e) => events.push(e));

    await agent.prompt('boom');
    client.fire({ type: 'text_delta', text: 'partial' });
    client.fire({ type: 'error', message: 'kernel exploded' });

    expect(agent.state.error).toBe('kernel exploded');
    expect(agent.state.isStreaming).toBe(false);
    expect(events.at(-1)!.type).toBe('agent_end');
    const errMsg = agent.state.messages.at(-1) as AssistantMessage;
    expect(errMsg.stopReason).toBe('error');
    expect(errMsg.errorMessage).toBe('kernel exploded');
  });

  it('reconnect_exhausted during a turn surfaces an error termination', async () => {
    const { agent, client } = makeAgent();
    const events: RaraAgentEvent[] = [];
    agent.subscribe((e) => events.push(e));

    await agent.prompt('hi');
    client.fireLifecycle({ type: 'closed', reason: 'reconnect_exhausted' });

    expect(events.at(-1)!.type).toBe('agent_end');
    expect(agent.state.error).toMatch(/reconnect/i);
  });

  it('appendMessage emits message_start/end without altering streaming state', () => {
    const { agent } = makeAgent();
    const events: RaraAgentEvent[] = [];
    agent.subscribe((e) => events.push(e));
    const msg = { role: 'user' as const, content: 'manual', timestamp: 1 };
    agent.appendMessage(msg);
    expect(events.map((e) => e.type)).toEqual(['message_start', 'message_end']);
    expect(agent.state.messages).toEqual([msg]);
    expect(agent.state.isStreaming).toBe(false);
  });

  it('observer callback receives every frame', async () => {
    const client = new FakeSessionWsClient();
    const observer = vi.fn();
    const agent = new RaraAgent({
      sessionId: 'sess-1',
      model: TEST_MODEL,
      clientFactory: () => client as unknown as SessionWsClient,
      observer,
    });
    await agent.prompt('hi');
    client.fire({ type: 'text_delta', text: 'a' });
    client.fire({ type: 'done' });

    const frameTypes = observer.mock.calls.map((c) => (c[1] as WebFrame).type);
    expect(frameTypes).toContain('hello');
    expect(frameTypes).toContain('text_delta');
    expect(frameTypes).toContain('done');
  });
});
