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

import type { AgentTool } from '@mariozechner/pi-agent-core';
import type { Context, Model } from '@mariozechner/pi-ai';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { buildWsUrl, createRaraStreamFn } from '../rara-stream';

const STORAGE_KEY = 'rara_backend_url';

// Node 22+ exposes a built-in `globalThis.localStorage` that shadows jsdom's
// implementation and lacks `setItem`/`getItem` unless launched with
// `--localstorage-file`. Tests here stub a minimal in-memory Storage so
// `buildWsUrl`'s override probe runs against predictable state regardless
// of the host Node version.
function installLocalStorageStub() {
  const store = new Map<string, string>();
  const stub = {
    getItem: (k: string) => (store.has(k) ? store.get(k)! : null),
    setItem: (k: string, v: string) => {
      store.set(k, String(v));
    },
    removeItem: (k: string) => {
      store.delete(k);
    },
    clear: () => store.clear(),
    key: (i: number) => Array.from(store.keys())[i] ?? null,
    get length() {
      return store.size;
    },
  };
  vi.stubGlobal('localStorage', stub);
  Object.defineProperty(window, 'localStorage', { value: stub, configurable: true });
}

describe('buildWsUrl — backend override resolution (#1622)', () => {
  beforeEach(() => {
    installLocalStorageStub();
    // Seed an authenticated principal + token so the WS URL builder does
    // not redirect to /login during these override-resolution tests.
    localStorage.setItem('access_token', 'test-token');
    localStorage.setItem(
      'auth_user',
      JSON.stringify({ user_id: 'alice', role: 'Admin', is_admin: true }),
    );
  });

  afterEach(() => {
    localStorage.removeItem(STORAGE_KEY);
    localStorage.removeItem('access_token');
    localStorage.removeItem('auth_user');
    vi.unstubAllGlobals();
  });

  it('falls back to window.location when no override is set', () => {
    const url = buildWsUrl('sess-abc');
    const loc = window.location;
    const proto = loc.protocol === 'https:' ? 'wss:' : 'ws:';
    expect(url).toBe(
      `${proto}//${loc.host}/api/v1/kernel/chat/ws?session_key=sess-abc&token=test-token`,
    );
  });

  it('honors rara_backend_url override (http -> ws)', () => {
    localStorage.setItem(STORAGE_KEY, 'http://10.0.0.183:25555');
    expect(buildWsUrl('sess-abc')).toBe(
      'ws://10.0.0.183:25555/api/v1/kernel/chat/ws?session_key=sess-abc&token=test-token',
    );
  });

  it('honors rara_backend_url override with https and trims trailing slash', () => {
    localStorage.setItem(STORAGE_KEY, 'https://backend.example.com/');
    expect(buildWsUrl('sess-xyz')).toBe(
      'wss://backend.example.com/api/v1/kernel/chat/ws?session_key=sess-xyz&token=test-token',
    );
  });

  it('URL-encodes session keys containing special characters', () => {
    localStorage.setItem(STORAGE_KEY, 'http://10.0.0.183:25555');
    expect(buildWsUrl('sess/with spaces')).toBe(
      'ws://10.0.0.183:25555/api/v1/kernel/chat/ws?session_key=sess%2Fwith+spaces&token=test-token',
    );
  });
});

// ---------------------------------------------------------------------------
// Relay Map stability across StreamFn invocations (#1732)
// ---------------------------------------------------------------------------

/**
 * Minimal mock WebSocket that exposes `onopen` / `onmessage` / `onclose`
 * callbacks so tests can drive the rara-stream state machine directly
 * without a live backend. Each constructed instance is tracked on
 * {@link MockWebSocket.instances} so the active test can reach in and
 * emit frames.
 */
class MockWebSocket {
  static instances: MockWebSocket[] = [];
  onopen: ((ev: Event) => void) | null = null;
  onmessage: ((ev: MessageEvent) => void) | null = null;
  onerror: ((ev: Event) => void) | null = null;
  onclose: ((ev: CloseEvent) => void) | null = null;
  sent: string[] = [];
  readyState = 1;
  url: string;
  constructor(url: string) {
    this.url = url;
    MockWebSocket.instances.push(this);
  }
  send(data: string) {
    this.sent.push(data);
  }
  close() {
    this.readyState = 3;
    this.onclose?.(new CloseEvent('close'));
  }
  emit(payload: unknown) {
    this.onmessage?.(new MessageEvent('message', { data: JSON.stringify(payload) }));
  }
}

function fakeModel(): Model<any> {
  return {
    id: 'test-model',
    api: 'test',
    provider: 'test',
    name: 'Test',
    baseUrl: 'http://test',
    contextWindow: 1000,
    maxTokens: 1000,
    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
  } as unknown as Model<any>;
}

function userContext(text: string): Context {
  return {
    systemPrompt: '',
    messages: [{ role: 'user', content: text }],
    tools: [],
  } as unknown as Context;
}

describe('createRaraStreamFn — relay Map stability across invocations (#1732)', () => {
  beforeEach(() => {
    installLocalStorageStub();
    // Seed an authenticated principal + token so `buildWsUrl` does not
    // redirect to /login before we get a chance to drive the WebSocket.
    localStorage.setItem('access_token', 'test-token');
    localStorage.setItem(
      'auth_user',
      JSON.stringify({ user_id: 'alice', role: 'Admin', is_admin: true }),
    );
    MockWebSocket.instances = [];
    vi.stubGlobal('WebSocket', MockWebSocket as unknown as typeof WebSocket);
  });

  afterEach(() => {
    localStorage.removeItem('access_token');
    localStorage.removeItem('auth_user');
    vi.unstubAllGlobals();
  });

  it('relay shim installed on first invocation resolves tool calls from a second invocation', async () => {
    const streamFn = createRaraStreamFn(() => 'sess-1');

    // --- First invocation: tool_call_start + tool_call_end for id "t1" ---
    const ctx = userContext('hello');
    void streamFn(fakeModel(), ctx);
    const ws1 = MockWebSocket.instances[0]!;
    ws1.onopen?.(new Event('open'));
    ws1.emit({
      type: 'tool_call_start',
      id: 't1',
      name: 'search',
      arguments: { q: 'first' },
    });
    ws1.emit({
      type: 'tool_call_end',
      id: 't1',
      result_preview: 'first-result',
      success: true,
      error: null,
    });
    ws1.emit({ type: 'done' });

    // Shim was installed into context.tools on first invocation.
    expect(ctx.tools?.map((t) => t.name)).toContain('search');
    const shim = ctx.tools?.find((t) => t.name === 'search') as AgentTool | undefined;
    expect(shim).toBeDefined();
    // First id resolves via the shim's execute().
    await expect(shim!.execute('t1', {}, {} as never)).resolves.toMatchObject({
      content: [{ type: 'text', text: 'first-result' }],
    });

    // --- Second invocation: new tool_call_start for a fresh id "t2" ---
    // pi-agent-core reuses the same `context` and its existing tool
    // entry, so rara-stream must not allocate a new pendingToolResults
    // Map — otherwise shim.execute('t2') throws "No kernel result ...".
    void streamFn(fakeModel(), ctx);
    const ws2 = MockWebSocket.instances[1]!;
    ws2.onopen?.(new Event('open'));
    ws2.emit({
      type: 'tool_call_start',
      id: 't2',
      name: 'search',
      arguments: { q: 'second' },
    });
    ws2.emit({
      type: 'tool_call_end',
      id: 't2',
      result_preview: 'second-result',
      success: true,
      error: null,
    });
    ws2.emit({ type: 'done' });

    // Same shim reference (not re-pushed) — confirms dedup across turns.
    const shim2 = ctx.tools?.find((t) => t.name === 'search') as AgentTool | undefined;
    expect(shim2).toBe(shim);
    expect(ctx.tools?.filter((t) => t.name === 'search').length).toBe(1);

    // Critical: the shim must resolve the new id from the shared Map.
    await expect(shim!.execute('t2', {}, {} as never)).resolves.toMatchObject({
      content: [{ type: 'text', text: 'second-result' }],
    });
  });
});

// ---------------------------------------------------------------------------
// WebSocket auto-reconnect (#1880)
// ---------------------------------------------------------------------------

describe('createRaraStreamFn — WebSocket auto-reconnect (#1880)', () => {
  beforeEach(() => {
    installLocalStorageStub();
    localStorage.setItem('access_token', 'test-token');
    localStorage.setItem(
      'auth_user',
      JSON.stringify({ user_id: 'alice', role: 'Admin', is_admin: true }),
    );
    MockWebSocket.instances = [];
    vi.stubGlobal('WebSocket', MockWebSocket as unknown as typeof WebSocket);
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
    localStorage.removeItem('access_token');
    localStorage.removeItem('auth_user');
    vi.unstubAllGlobals();
  });

  it('emits __stream_reconnecting and reconnects after onclose without done', () => {
    const events: { type: string }[] = [];
    const streamFn = createRaraStreamFn(
      () => 'sess-recon',
      undefined,
      (_sk, ev) => events.push(ev),
    );
    void streamFn(fakeModel(), userContext('hi'));
    const ws1 = MockWebSocket.instances[0]!;
    ws1.onopen?.(new Event('open'));
    expect(events.some((e) => e.type === '__stream_started')).toBe(true);

    // Simulate transport drop (socket close without a `done` first).
    ws1.onclose?.(new CloseEvent('close'));
    // The reconnecting frame should fire synchronously on close.
    expect(events.find((e) => e.type === '__stream_reconnecting')).toBeDefined();
    // No __stream_closed yet — we are mid-grace window.
    expect(events.some((e) => e.type === '__stream_closed')).toBe(false);

    // Advance past first backoff (250ms) — a fresh socket should open.
    vi.advanceTimersByTime(300);
    expect(MockWebSocket.instances).toHaveLength(2);
    const ws2 = MockWebSocket.instances[1]!;
    ws2.onopen?.(new Event('open'));
    // Reconnect must NOT re-send the user payload (backend has buffered).
    expect(ws2.sent).toHaveLength(0);

    // Backend resumes and finishes — observer eventually sees done + closed.
    ws2.emit({ type: 'done' });
    expect(events.some((e) => e.type === '__stream_closed')).toBe(true);
  });

  it('gives up after MAX_RECONNECT_ATTEMPTS and emits __stream_reconnect_failed', () => {
    const events: { type: string }[] = [];
    const streamFn = createRaraStreamFn(
      () => 'sess-fail',
      undefined,
      (_sk, ev) => events.push(ev),
    );
    void streamFn(fakeModel(), userContext('hi'));

    // Drive 5 failed reconnect cycles. Backoffs are [250, 500, 1000, 2000, 4000].
    const backoffs = [250, 500, 1_000, 2_000, 4_000];
    let socketIdx = 0;
    {
      const ws = MockWebSocket.instances[socketIdx]!;
      ws.onopen?.(new Event('open'));
      ws.onclose?.(new CloseEvent('close'));
      socketIdx += 1;
    }
    for (const delay of backoffs) {
      vi.advanceTimersByTime(delay + 1);
      const ws = MockWebSocket.instances[socketIdx];
      if (!ws) break;
      // Reconnect attempt opens but immediately drops without ever
      // succeeding — exhausting the budget.
      ws.onopen?.(new Event('open'));
      ws.onclose?.(new CloseEvent('close'));
      socketIdx += 1;
    }

    // After the 5th failed retry, the next onclose should emit reconnect_failed.
    expect(events.find((e) => e.type === '__stream_reconnect_failed')).toBeDefined();
    expect(events.find((e) => e.type === '__stream_closed')).toBeDefined();
  });

  it('does not reconnect after a clean done frame', () => {
    const events: { type: string }[] = [];
    const streamFn = createRaraStreamFn(
      () => 'sess-done',
      undefined,
      (_sk, ev) => events.push(ev),
    );
    void streamFn(fakeModel(), userContext('hi'));
    const ws1 = MockWebSocket.instances[0]!;
    ws1.onopen?.(new Event('open'));
    ws1.emit({ type: 'done' });
    // `done` calls ws.close() which fires onclose. Verify no retry.
    vi.advanceTimersByTime(10_000);
    expect(MockWebSocket.instances).toHaveLength(1);
    expect(events.some((e) => e.type === '__stream_reconnecting')).toBe(false);
  });
});
