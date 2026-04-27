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
import { act } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { useSessionEvents } from '../use-session-events';

vi.mock('@/api/client', () => ({
  getAccessToken: () => 'test-token',
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
const sockets: MockWebSocket[] = [];

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
    sockets.push(this);
  }

  close() {
    this.closed = true;
  }
}

describe('useSessionEvents', () => {
  beforeEach(() => {
    lastSocket = null;
    sockets.length = 0;
    vi.useFakeTimers();
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
    vi.useRealTimers();
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

  it('stops reconnecting after the retry budget is exhausted', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const onTapeAppended = vi.fn();
    renderHook(() => useSessionEvents({ sessionKey: 'sess-abc', onTapeAppended }));

    // Initial connect + 5 retries = 6 sockets total. The 6th close exhausts.
    for (let i = 0; i < 6; i += 1) {
      const sock = lastSocket!;
      act(() => {
        sock.onclose?.(new CloseEvent('close'));
      });
      act(() => {
        vi.runOnlyPendingTimers();
      });
    }

    expect(sockets.length).toBe(6);
    expect(warnSpy).toHaveBeenCalledTimes(1);
  });

  it('resets the retry budget when a hello frame arrives', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const onTapeAppended = vi.fn();
    renderHook(() => useSessionEvents({ sessionKey: 'sess-abc', onTapeAppended }));

    // Burn 4 of the 5 retries with immediate closes.
    for (let i = 0; i < 4; i += 1) {
      const sock = lastSocket!;
      act(() => {
        sock.onclose?.(new CloseEvent('close'));
      });
      act(() => {
        vi.runOnlyPendingTimers();
      });
    }
    // 5 sockets so far (1 initial + 4 retries).
    expect(sockets.length).toBe(5);

    // Hello frame on the live socket resets the budget.
    act(() => {
      lastSocket!.onmessage?.(
        new MessageEvent('message', { data: JSON.stringify({ type: 'hello' }) }),
      );
    });

    // Now we should be able to absorb a full new budget of 5 retries.
    for (let i = 0; i < 5; i += 1) {
      const sock = lastSocket!;
      act(() => {
        sock.onclose?.(new CloseEvent('close'));
      });
      act(() => {
        vi.runOnlyPendingTimers();
      });
    }
    expect(sockets.length).toBe(10); // 5 pre-hello + 5 post-hello retries.
    expect(warnSpy).not.toHaveBeenCalled();

    // One more close past the new budget triggers the warn.
    act(() => {
      lastSocket!.onclose?.(new CloseEvent('close'));
    });
    act(() => {
      vi.runOnlyPendingTimers();
    });
    expect(sockets.length).toBe(10);
    expect(warnSpy).toHaveBeenCalledTimes(1);
  });

  it('cancels pending retries when sessionKey changes', () => {
    const onTapeAppended = vi.fn();
    const { rerender } = renderHook(
      ({ key }: { key: string | null }) => useSessionEvents({ sessionKey: key, onTapeAppended }),
      { initialProps: { key: 'sess-abc' as string | null } },
    );

    // Trigger a close so a retry is queued for sess-abc.
    act(() => {
      lastSocket!.onclose?.(new CloseEvent('close'));
    });
    const beforeSwitch = sockets.length;

    // Switch session — the queued retry must NOT fire for sess-abc.
    rerender({ key: 'sess-xyz' });
    act(() => {
      vi.runOnlyPendingTimers();
    });

    // After rerender, exactly one new socket opens for sess-xyz.
    expect(sockets.length).toBe(beforeSwitch + 1);
    expect(lastSocket!.url).toContain('sess-xyz');
  });
});
