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

import { renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { useSessionEvents } from '../use-session-events';

vi.mock('@/api/client', () => ({
  getAccessToken: () => 'test-token',
}));

vi.mock('@/adapters/ws-base-url', () => ({
  buildWsBaseUrl: () => 'ws://localhost:5173',
}));

interface MockWebSocket {
  url: string;
  onopen: ((ev: Event) => void) | null;
  onmessage: ((ev: MessageEvent) => void) | null;
  onerror: ((ev: Event) => void) | null;
  onclose: ((ev: CloseEvent) => void) | null;
  close: () => void;
}

let lastSocket: MockWebSocket | null = null;

class FakeWebSocket implements MockWebSocket {
  url: string;
  onopen: ((ev: Event) => void) | null = null;
  onmessage: ((ev: MessageEvent) => void) | null = null;
  onerror: ((ev: Event) => void) | null = null;
  onclose: ((ev: CloseEvent) => void) | null = null;
  closed = false;

  constructor(url: string) {
    this.url = url;
    // eslint-disable-next-line @typescript-eslint/no-this-alias
    lastSocket = this;
  }

  close() {
    this.closed = true;
  }
}

describe('useSessionEvents', () => {
  beforeEach(() => {
    lastSocket = null;
    Object.defineProperty(globalThis, 'WebSocket', {
      writable: true,
      configurable: true,
      value: FakeWebSocket,
    });
    Object.defineProperty(globalThis, 'window', {
      writable: true,
      configurable: true,
      value: {
        location: { host: 'localhost:5173', protocol: 'http:' },
      },
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('opens a WebSocket scoped to the session key', () => {
    const onTapeAppended = vi.fn();
    renderHook(() => useSessionEvents({ sessionKey: 'sess-abc', onTapeAppended }));
    expect(lastSocket).not.toBeNull();
    expect(lastSocket!.url).toContain('/api/v1/kernel/chat/events/sess-abc');
    expect(lastSocket!.url).toContain('token=test-token');
  });

  it('invokes onTapeAppended when a tape_appended frame arrives', () => {
    const onTapeAppended = vi.fn();
    renderHook(() => useSessionEvents({ sessionKey: 'sess-abc', onTapeAppended }));
    const sock = lastSocket!;

    sock.onmessage?.(
      new MessageEvent('message', {
        data: JSON.stringify({
          type: 'tape_appended',
          entry_id: 42,
          role: 'assistant',
          timestamp: '2026-01-01T00:00:00Z',
        }),
      }),
    );

    expect(onTapeAppended).toHaveBeenCalledWith({
      entry_id: 42,
      role: 'assistant',
      timestamp: '2026-01-01T00:00:00Z',
    });
  });

  it('ignores hello frames and unknown types', () => {
    const onTapeAppended = vi.fn();
    renderHook(() => useSessionEvents({ sessionKey: 'sess-abc', onTapeAppended }));
    const sock = lastSocket!;

    sock.onmessage?.(new MessageEvent('message', { data: JSON.stringify({ type: 'hello' }) }));
    sock.onmessage?.(new MessageEvent('message', { data: JSON.stringify({ type: 'unknown' }) }));
    sock.onmessage?.(new MessageEvent('message', { data: 'not-json' }));

    expect(onTapeAppended).not.toHaveBeenCalled();
  });

  it('does not open a socket when sessionKey is null', () => {
    const onTapeAppended = vi.fn();
    renderHook(() => useSessionEvents({ sessionKey: null, onTapeAppended }));
    expect(lastSocket).toBeNull();
  });
});
