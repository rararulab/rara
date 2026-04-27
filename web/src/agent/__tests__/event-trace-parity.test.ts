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
 * Event-trace parity contract test — the phase (c) canary for #1935.
 *
 * `<agent-interface>` (pi-web-ui's `AgentInterface.ts:153-186`) only acts on
 * a fixed subset of event types: `agent_start`, `turn_start`, `message_start`,
 * `message_update`, `message_end`, `turn_end`, `agent_end`. Anything else
 * the agent emits is dead weight; anything missing breaks the host. This
 * test pins RaraAgent's emission shape to that contract for the three
 * canonical assistant turns (text-only / with-tool / errored) so contract
 * drift is caught here and not at runtime in phase (d) wiring.
 */

import type { Api, Model } from '@mariozechner/pi-ai';
import { describe, expect, it } from 'vitest';

import { RaraAgent } from '@/agent/rara-agent';
import {
  type FrameHandler,
  type LifecycleHandler,
  SessionWsClient,
  type WebFrame,
} from '@/agent/session-ws-client';
import type { RaraAgentEvent } from '@/agent/types';

// The seven event types `<agent-interface>` reads. Anything outside this set
// is silently ignored by the host — emitting one is harmless but emitting an
// unknown variant in place of one of these breaks streaming.
const HOST_EVENT_TYPES = new Set<RaraAgentEvent['type']>([
  'agent_start',
  'turn_start',
  'message_start',
  'message_update',
  'message_end',
  'turn_end',
  'agent_end',
]);

class FakeClient {
  frameHandlers: FrameHandler[] = [];
  lifecycleHandlers: LifecycleHandler[] = [];
  connected = false;
  onFrame(h: FrameHandler) {
    this.frameHandlers.push(h);
    return () => {};
  }
  onLifecycle(h: LifecycleHandler) {
    this.lifecycleHandlers.push(h);
    return () => {};
  }
  connect() {
    this.connected = true;
    this.fire({ type: 'hello' });
  }
  disconnect() {
    this.connected = false;
  }
  prompt() {
    return this.connected;
  }
  abort() {
    return this.connected;
  }
  fire(frame: WebFrame) {
    for (const h of this.frameHandlers) h(frame);
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

function setup() {
  const client = new FakeClient();
  const agent = new RaraAgent({
    sessionId: 'sess-1',
    model: TEST_MODEL,
    clientFactory: () => client as unknown as SessionWsClient,
  });
  const events: RaraAgentEvent[] = [];
  agent.subscribe((e) => events.push(e));
  return { agent, client, events };
}

/** Project an event trace to just its `.type` field for ordering assertions. */
function types(events: RaraAgentEvent[]): string[] {
  return events.map((e) => e.type);
}

describe('RaraAgent ↔ <agent-interface> event-trace contract', () => {
  it('text-only turn: emits the documented seven-stage trace', async () => {
    const { agent, client, events } = setup();

    await agent.prompt('hi');
    client.fire({ type: 'text_delta', text: 'hel' });
    client.fire({ type: 'text_delta', text: 'lo' });
    client.fire({ type: 'done' });

    expect(types(events)).toEqual([
      'agent_start',
      'turn_start',
      'message_start', // user
      'message_end', // user
      'message_start', // assistant partial
      'message_update', // text_delta #1
      'message_update', // text_delta #2
      'message_end', // assistant final
      'turn_end',
      'agent_end',
    ]);

    // Every emitted event type is one the host actually reads.
    for (const e of events) {
      expect(HOST_EVENT_TYPES.has(e.type)).toBe(true);
    }

    // message_update events MUST carry `.message` — the host reads it at
    // AgentInterface.ts:182 (`setMessage(ev.message, ...)`). Missing this
    // field is the most likely silent contract break.
    const updates = events.filter((e) => e.type === 'message_update');
    expect(updates).toHaveLength(2);
    for (const ev of updates) {
      if (ev.type !== 'message_update') throw new Error('unreachable');
      expect(ev.message).toBeDefined();
      expect(ev.message.role).toBe('assistant');
    }

    // turn_end carries the final assistant message + (empty) tool results.
    const turnEnd = events.find((e) => e.type === 'turn_end');
    if (turnEnd?.type !== 'turn_end') throw new Error('missing turn_end');
    expect(turnEnd.message.role).toBe('assistant');
    expect(turnEnd.toolResults).toEqual([]);

    // agent_end carries the final messages snapshot.
    const agentEnd = events.find((e) => e.type === 'agent_end');
    if (agentEnd?.type !== 'agent_end') throw new Error('missing agent_end');
    expect(agentEnd.messages).toHaveLength(2);
  });

  it('tool-call turn: assistant message_end precedes tool-result message pairs and turn_end', async () => {
    const { agent, client, events } = setup();

    await agent.prompt('use a tool');
    client.fire({
      type: 'tool_call_start',
      id: 't1',
      name: 'search',
      arguments: { q: 'x' },
    });
    client.fire({
      type: 'tool_call_end',
      id: 't1',
      result_preview: 'found',
      success: true,
      error: null,
    });
    client.fire({ type: 'done' });

    expect(types(events)).toEqual([
      'agent_start',
      'turn_start',
      'message_start', // user
      'message_end', // user
      'message_start', // assistant partial
      'message_update', // tool_call_start (toolcall_start AssistantMessageEvent)
      'message_update', // tool_call_end (toolcall_end AssistantMessageEvent)
      'message_end', // assistant final
      'message_start', // toolResult
      'message_end', // toolResult
      'turn_end',
      'agent_end',
    ]);

    for (const e of events) {
      expect(HOST_EVENT_TYPES.has(e.type)).toBe(true);
    }

    const turnEnd = events.find((e) => e.type === 'turn_end');
    if (turnEnd?.type !== 'turn_end') throw new Error('unreachable');
    expect(turnEnd.toolResults).toHaveLength(1);
    expect(turnEnd.toolResults[0]!.toolCallId).toBe('t1');

    // The toolResult message_start must precede turn_end so the host's
    // stable list contains the tool result before it strips the streaming
    // container on agent_end.
    const indexes = events.map((e, i) => ({ type: e.type, i }));
    const lastToolResEnd = indexes.filter((x) => x.type === 'message_end').at(-1);
    const turnEndIdx = indexes.find((x) => x.type === 'turn_end');
    expect(lastToolResEnd!.i).toBeLessThan(turnEndIdx!.i);
  });

  it('error turn: still emits a complete agent_start..agent_end envelope', async () => {
    const { agent, client, events } = setup();

    await agent.prompt('boom');
    client.fire({ type: 'text_delta', text: 'partial' });
    client.fire({ type: 'error', message: 'kernel error' });

    const t = types(events);
    expect(t[0]).toBe('agent_start');
    expect(t.at(-1)).toBe('agent_end');
    // turn_end must precede agent_end so the host clears the streaming
    // container before the final list render.
    expect(t.lastIndexOf('turn_end')).toBeLessThan(t.lastIndexOf('agent_end'));

    for (const e of events) {
      expect(HOST_EVENT_TYPES.has(e.type)).toBe(true);
    }

    const turnEnd = events.find((e) => e.type === 'turn_end');
    if (turnEnd?.type !== 'turn_end') throw new Error('unreachable');
    expect(turnEnd.message.role).toBe('assistant');
    expect(turnEnd.toolResults).toEqual([]);
  });
});
