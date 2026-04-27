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

import { AUTO_DISMISS_MS, LiveRunStore, mergeBySourceSeq, timelineKey } from '../live-run-store';

import type { PublicWebEvent } from '@/agent/session-ws-client';
import type { TimelineItem } from '@/api/kernel-types';

const startEvent = { type: '__stream_started' } satisfies PublicWebEvent;
const closeEvent = { type: '__stream_closed' } satisfies PublicWebEvent;
const reconnectingEvent = (attempt = 1, delayMs = 250) =>
  ({ type: '__stream_reconnecting', attempt, delayMs }) satisfies PublicWebEvent;
const reconnectFailedEvent = (attempts = 5) =>
  ({ type: '__stream_reconnect_failed', attempts }) satisfies PublicWebEvent;

function toolStart(id: string, name: string, args: Record<string, unknown> = {}): PublicWebEvent {
  return { type: 'tool_call_start', id, name, arguments: args } satisfies PublicWebEvent;
}

function toolEnd(
  id: string,
  preview: string,
  opts: { success?: boolean; error?: string | null } = {},
): PublicWebEvent {
  return {
    type: 'tool_call_end',
    id,
    result_preview: preview,
    success: opts.success ?? true,
    error: opts.error ?? null,
  } satisfies PublicWebEvent;
}

describe('LiveRunStore', () => {
  it('assigns monotonic seqs within a run and preserves arrival order', () => {
    const store = new LiveRunStore();
    const sk = 's1';
    store.publish(sk, startEvent);
    store.publish(sk, toolStart('a', 'Grep'));
    store.publish(sk, toolEnd('a', 'found 3 matches'));
    store.publish(sk, toolStart('b', 'Read'));
    store.publish(sk, toolEnd('b', 'file contents'));
    const run = store.snapshot(sk).active!;
    expect(run).not.toBeNull();
    const seqs = run.items.map((it) => it.seq);
    expect(seqs).toEqual([0, 1, 2, 3]);
    // Kinds should alternate tool_use / tool_result in arrival order.
    expect(run.items.map((it) => it.kind)).toEqual([
      'tool_use',
      'tool_result',
      'tool_use',
      'tool_result',
    ]);
  });

  it('dedupes a replayed tool_call_start (out-of-order delivery)', () => {
    const store = new LiveRunStore();
    const sk = 's2';
    store.publish(sk, startEvent);
    store.publish(sk, toolStart('a', 'Grep'));
    // Replay the same start — should not create a second row or bump toolCalls.
    store.publish(sk, toolStart('a', 'Grep'));
    const run = store.snapshot(sk).active!;
    const useItems = run.items.filter((it) => it.kind === 'tool_use');
    expect(useItems).toHaveLength(1);
    expect(run.toolCalls).toBe(1);
  });

  it('keeps a completed run pinned in the active slot after done', () => {
    const store = new LiveRunStore();
    const sk = 's3';
    store.publish(sk, startEvent);
    store.publish(sk, toolStart('a', 'Grep'));
    store.publish(sk, toolEnd('a', 'done'));
    store.publish(sk, { type: 'done' } satisfies PublicWebEvent);
    const slice = store.snapshot(sk);
    expect(slice.active?.status).toBe('completed');
    expect(slice.history).toHaveLength(0);
  });

  it('retires the completed run to history when the next stream starts', () => {
    const store = new LiveRunStore();
    const sk = 's3b';
    store.publish(sk, startEvent);
    store.publish(sk, { type: 'done' } satisfies PublicWebEvent);
    store.publish(sk, startEvent);
    const slice = store.snapshot(sk);
    expect(slice.active?.status).toBe('running');
    expect(slice.history).toHaveLength(1);
    expect(slice.history[0]?.status).toBe('completed');
  });

  it('marks a stream_closed without done as cancelled and keeps it visible', () => {
    const store = new LiveRunStore();
    const sk = 's4';
    store.publish(sk, startEvent);
    store.publish(sk, toolStart('a', 'Grep'));
    store.publish(sk, closeEvent);
    const slice = store.snapshot(sk);
    expect(slice.active?.status).toBe('cancelled');
    expect(slice.history).toHaveLength(0);
  });

  it('tracks the latest progress.stage on the active run', () => {
    const store = new LiveRunStore();
    const sk = 's5';
    store.publish(sk, startEvent);
    expect(store.snapshot(sk).active?.currentStage).toBeNull();
    store.publish(sk, { type: 'progress', stage: 'thinking' });
    expect(store.snapshot(sk).active?.currentStage).toBe('thinking');
    store.publish(sk, {
      type: 'progress',
      stage: 'Waiting for LLM response (iteration 2)...',
    });
    expect(store.snapshot(sk).active?.currentStage).toBe(
      'Waiting for LLM response (iteration 2)...',
    );
  });
});

describe('LiveRunStore auto-dismiss', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it('retires a completed run to history after AUTO_DISMISS_MS', () => {
    const store = new LiveRunStore();
    const sk = 'auto1';
    store.publish(sk, startEvent);
    store.publish(sk, { type: 'done' } satisfies PublicWebEvent);
    expect(store.snapshot(sk).active?.status).toBe('completed');

    vi.advanceTimersByTime(AUTO_DISMISS_MS - 1);
    expect(store.snapshot(sk).active?.status).toBe('completed');

    vi.advanceTimersByTime(1);
    const slice = store.snapshot(sk);
    expect(slice.active).toBeNull();
    expect(slice.history).toHaveLength(1);
    expect(slice.history[0]?.status).toBe('completed');
  });

  it('retires a failed run after the same delay', () => {
    const store = new LiveRunStore();
    const sk = 'auto2';
    store.publish(sk, startEvent);
    store.publish(sk, { type: 'error', message: 'boom' } satisfies PublicWebEvent);
    expect(store.snapshot(sk).active?.status).toBe('failed');

    vi.advanceTimersByTime(AUTO_DISMISS_MS);
    const slice = store.snapshot(sk);
    expect(slice.active).toBeNull();
    expect(slice.history[0]?.status).toBe('failed');
  });

  it('cancels the dismiss timer when a new stream starts', () => {
    const store = new LiveRunStore();
    const sk = 'auto3';
    store.publish(sk, startEvent);
    store.publish(sk, { type: 'done' } satisfies PublicWebEvent);
    vi.advanceTimersByTime(AUTO_DISMISS_MS - 100);
    // New stream lands before the timer fires — it should retire the
    // completed run via the existing __stream_started path and not
    // double-retire when the timer would have fired.
    store.publish(sk, startEvent);
    expect(store.snapshot(sk).active?.status).toBe('running');
    expect(store.snapshot(sk).history).toHaveLength(1);

    vi.advanceTimersByTime(1_000);
    // Active is still the running run; history did not gain a duplicate.
    expect(store.snapshot(sk).active?.status).toBe('running');
    expect(store.snapshot(sk).history).toHaveLength(1);
  });

  it('does not arm a timer while the run is still running', () => {
    const store = new LiveRunStore();
    const sk = 'auto4';
    store.publish(sk, startEvent);
    store.publish(sk, toolStart('a', 'Grep'));
    vi.advanceTimersByTime(AUTO_DISMISS_MS * 2);
    expect(store.snapshot(sk).active?.status).toBe('running');
  });

  it('reset clears any pending dismiss timer', () => {
    const store = new LiveRunStore();
    const sk = 'auto5';
    store.publish(sk, startEvent);
    store.publish(sk, { type: 'done' } satisfies PublicWebEvent);
    store.reset(sk);
    // Even after the dismiss window, no spurious mutation occurs.
    vi.advanceTimersByTime(AUTO_DISMISS_MS);
    expect(store.snapshot(sk).active).toBeNull();
    expect(store.snapshot(sk).history).toHaveLength(0);
  });
});

describe('LiveRunStore reconnect grace period (#1880)', () => {
  it('flips to reconnecting on __stream_reconnecting without finalizing', () => {
    const store = new LiveRunStore();
    const sk = 'rec1';
    store.publish(sk, startEvent);
    store.publish(sk, toolStart('a', 'Grep'));
    store.publish(sk, reconnectingEvent(1, 250));
    const slice = store.snapshot(sk);
    expect(slice.active?.status).toBe('reconnecting');
    expect(slice.active?.endedAt).toBeNull();
    // Items survive — the user can still see what ran before the drop.
    expect(slice.active?.items.length).toBeGreaterThan(0);
  });

  it('resumes to running when a backend frame arrives after reconnecting', () => {
    const store = new LiveRunStore();
    const sk = 'rec2';
    store.publish(sk, startEvent);
    store.publish(sk, reconnectingEvent());
    expect(store.snapshot(sk).active?.status).toBe('reconnecting');
    // Resumed socket delivers a tool_call_start — status flips back.
    store.publish(sk, toolStart('a', 'Grep'));
    expect(store.snapshot(sk).active?.status).toBe('running');
  });

  it('finalizes as completed if reconnected stream ends with done', () => {
    const store = new LiveRunStore();
    const sk = 'rec3';
    store.publish(sk, startEvent);
    store.publish(sk, reconnectingEvent());
    store.publish(sk, { type: 'done' } satisfies PublicWebEvent);
    expect(store.snapshot(sk).active?.status).toBe('completed');
  });

  it('flips to failed only after __stream_reconnect_failed (not on bare close)', () => {
    const store = new LiveRunStore();
    const sk = 'rec4';
    store.publish(sk, startEvent);
    store.publish(sk, reconnectingEvent());
    // A spurious __stream_closed mid-reconnect must NOT mark failed.
    store.publish(sk, closeEvent);
    expect(store.snapshot(sk).active?.status).toBe('reconnecting');
    // Now backoff exhausts.
    store.publish(sk, reconnectFailedEvent(5));
    const slice = store.snapshot(sk);
    expect(slice.active?.status).toBe('failed');
    expect(slice.active?.error).toContain('reconnect failed');
  });
});

describe('mergeBySourceSeq', () => {
  it('keeps historical and live seqs separate by source key', () => {
    const h: TimelineItem[] = [
      { seq: 0, turn: 0, kind: 'agent', content: 'h0' },
      { seq: 1, turn: 0, kind: 'agent', content: 'h1' },
    ];
    const l: TimelineItem[] = [
      // Same seq as historical — MUST NOT collide because source differs.
      { seq: 0, turn: 0, kind: 'thinking', content: 'l0' },
      { seq: 1, turn: 0, kind: 'thinking', content: 'l1' },
    ];
    const merged = mergeBySourceSeq(h, l);
    expect(merged).toHaveLength(4);
    // Order: historical first, then live — both monotonic within source.
    expect(merged.map((it) => `${it.kind}:${it.seq}`)).toEqual([
      'agent:0',
      'agent:1',
      'thinking:0',
      'thinking:1',
    ]);
  });

  it('timelineKey composes source + seq into a unique string', () => {
    expect(timelineKey('h', 3)).toBe('h-3');
    expect(timelineKey('l', 3)).toBe('l-3');
    expect(timelineKey('h', 3)).not.toBe(timelineKey('l', 3));
  });
});
