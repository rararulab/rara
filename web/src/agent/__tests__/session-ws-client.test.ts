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

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { type LifecycleEvent, SessionWsClient, type WebFrame } from '@/agent/session-ws-client';

// ---------------------------------------------------------------------------
// FakeWebSocket — drop-in for the global `WebSocket` ctor used by the client.
// Tests can drive `onmessage`, `onclose`, `onerror` deterministically.
// ---------------------------------------------------------------------------

class FakeWebSocket {
  static instances: FakeWebSocket[] = [];
  readyState = 0;
  onopen: ((ev: Event) => void) | null = null;
  onmessage: ((ev: MessageEvent) => void) | null = null;
  onerror: ((ev: Event) => void) | null = null;
  onclose: ((ev: CloseEvent) => void) | null = null;
  sent: string[] = [];
  url: string;

  constructor(url: string) {
    this.url = url;
    FakeWebSocket.instances.push(this);
  }

  send(data: string): void {
    this.sent.push(data);
  }

  close(): void {
    this.readyState = 3;
  }

  // Test helpers ----------------------------------------------------------
  fireOpen(): void {
    this.readyState = 1;
    this.onopen?.(new Event('open'));
  }

  fireMessage(frame: WebFrame): void {
    this.onmessage?.(new MessageEvent('message', { data: JSON.stringify(frame) }));
  }

  fireClose(): void {
    this.readyState = 3;
    this.onclose?.(new CloseEvent('close'));
  }
}

vi.mock('@/api/client', () => ({
  getAccessToken: () => 'test-token',
}));
vi.mock('@/adapters/ws-base-url', () => ({
  buildWsBaseUrl: () => 'ws://localhost:5173',
}));

describe('SessionWsClient', () => {
  beforeEach(() => {
    FakeWebSocket.instances.length = 0;
    vi.useFakeTimers();
    (globalThis as unknown as { WebSocket: typeof FakeWebSocket }).WebSocket = FakeWebSocket;
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('connects and emits `connected` only after `hello` frame', () => {
    const client = new SessionWsClient({ sessionKey: 'sess-1' });
    const lifecycle: LifecycleEvent[] = [];
    client.onLifecycle((e) => lifecycle.push(e));

    client.connect();
    const ws = FakeWebSocket.instances[0]!;
    expect(ws.url).toContain('/api/v1/kernel/chat/session/sess-1');
    expect(ws.url).toContain('token=test-token');

    ws.fireOpen();
    expect(lifecycle).toEqual([]); // open alone is not proof-of-life

    ws.fireMessage({ type: 'hello' });
    expect(lifecycle).toEqual([{ type: 'connected' }]);
  });

  it('forwards frames to onFrame listeners', () => {
    const client = new SessionWsClient({ sessionKey: 'sess-1' });
    const frames: WebFrame[] = [];
    client.onFrame((f) => frames.push(f));
    client.connect();
    const ws = FakeWebSocket.instances[0]!;
    ws.fireOpen();

    ws.fireMessage({ type: 'hello' });
    ws.fireMessage({ type: 'text_delta', text: 'hi' });
    ws.fireMessage({ type: 'done' });

    expect(frames).toEqual([
      { type: 'hello' },
      { type: 'text_delta', text: 'hi' },
      { type: 'done' },
    ]);
  });

  it('disconnect emits closed{user} and stops reconnecting', () => {
    const client = new SessionWsClient({ sessionKey: 'sess-1' });
    const lifecycle: LifecycleEvent[] = [];
    client.onLifecycle((e) => lifecycle.push(e));
    client.connect();
    const ws = FakeWebSocket.instances[0]!;
    ws.fireOpen();
    ws.fireMessage({ type: 'hello' });

    client.disconnect();

    expect(lifecycle.at(-1)).toEqual({ type: 'closed', reason: 'user' });
    // After disconnect, a stray close should not schedule a reconnect.
    ws.fireClose();
    vi.advanceTimersByTime(10_000);
    expect(FakeWebSocket.instances).toHaveLength(1);
  });

  it('reconnects with exponential backoff and exhausts after 5 attempts', () => {
    const client = new SessionWsClient({ sessionKey: 'sess-1' });
    const lifecycle: LifecycleEvent[] = [];
    client.onLifecycle((e) => lifecycle.push(e));
    client.connect();

    // Drop the initial socket without ever sending hello.
    FakeWebSocket.instances[0]!.fireClose();

    const expectedDelays = [250, 500, 1_000, 2_000, 4_000];
    for (let i = 0; i < expectedDelays.length; i++) {
      const ev = lifecycle.at(-1)!;
      expect(ev).toEqual({ type: 'reconnecting', attempt: i + 1, delayMs: expectedDelays[i] });
      vi.advanceTimersByTime(expectedDelays[i]!);
      expect(FakeWebSocket.instances).toHaveLength(i + 2);
      FakeWebSocket.instances.at(-1)!.fireClose();
    }

    // After 5 failed retries, the client gives up.
    expect(lifecycle.at(-1)).toEqual({ type: 'closed', reason: 'reconnect_exhausted' });
  });

  it('hello frame resets the retry budget', () => {
    const client = new SessionWsClient({ sessionKey: 'sess-1' });
    const lifecycle: LifecycleEvent[] = [];
    client.onLifecycle((e) => lifecycle.push(e));
    client.connect();

    // First socket: connect, hello, then drop.
    FakeWebSocket.instances[0]!.fireOpen();
    FakeWebSocket.instances[0]!.fireMessage({ type: 'hello' });
    FakeWebSocket.instances[0]!.fireClose();

    // First reconnect uses the FIRST backoff slot (250ms) — proof the budget reset.
    expect(lifecycle.at(-1)).toEqual({ type: 'reconnecting', attempt: 1, delayMs: 250 });
  });

  it('prompt() sends a JSON frame when socket is open', () => {
    const client = new SessionWsClient({ sessionKey: 'sess-1' });
    client.connect();
    const ws = FakeWebSocket.instances[0]!;
    ws.fireOpen();
    ws.fireMessage({ type: 'hello' });

    expect(client.prompt('hello world')).toBe(true);
    expect(JSON.parse(ws.sent[0]!)).toEqual({ type: 'prompt', content: 'hello world' });
  });

  it('prompt() returns false when socket is not open', () => {
    const client = new SessionWsClient({ sessionKey: 'sess-1' });
    client.connect();
    // Don't fire open — readyState stays 0.
    expect(client.prompt('hi')).toBe(false);
  });

  it('abort() sends an abort frame', () => {
    const client = new SessionWsClient({ sessionKey: 'sess-1' });
    client.connect();
    const ws = FakeWebSocket.instances[0]!;
    ws.fireOpen();
    ws.fireMessage({ type: 'hello' });

    client.abort();
    expect(JSON.parse(ws.sent[0]!)).toEqual({ type: 'abort' });
  });

  it('parses every documented frame variant without crashing', () => {
    const client = new SessionWsClient({ sessionKey: 'sess-1' });
    const frames: WebFrame[] = [];
    client.onFrame((f) => frames.push(f));
    client.connect();
    const ws = FakeWebSocket.instances[0]!;
    ws.fireOpen();

    const variants: WebFrame[] = [
      { type: 'hello' },
      { type: 'message', content: 'x' },
      { type: 'typing' },
      { type: 'phase', phase: 'reasoning' },
      { type: 'error', message: 'boom' },
      { type: 'text_delta', text: 'a' },
      { type: 'reasoning_delta', text: 'b' },
      { type: 'text_clear' },
      { type: 'tool_call_start', name: 'echo', id: 't1', arguments: {} },
      {
        type: 'tool_call_end',
        id: 't1',
        result_preview: 'ok',
        success: true,
        error: null,
      },
      { type: 'turn_rationale', text: 'r' },
      { type: 'progress', stage: 's' },
      {
        type: 'turn_metrics',
        duration_ms: 1,
        iterations: 1,
        tool_calls: 0,
        model: 'm',
      },
      {
        type: 'usage',
        input: 1,
        output: 1,
        cache_read: 0,
        cache_write: 0,
        total_tokens: 2,
        cost: 0,
        model: 'm',
      },
      { type: 'done' },
      { type: 'tape_appended', entry_id: 1, role: 'user', timestamp: 'now' },
    ];
    for (const f of variants) ws.fireMessage(f);
    expect(frames).toHaveLength(variants.length);
  });

  it('ignores non-JSON messages instead of crashing', () => {
    const client = new SessionWsClient({ sessionKey: 'sess-1' });
    const frames: WebFrame[] = [];
    client.onFrame((f) => frames.push(f));
    client.connect();
    const ws = FakeWebSocket.instances[0]!;
    ws.onmessage?.(new MessageEvent('message', { data: 'not json' }));
    expect(frames).toEqual([]);
  });
});
