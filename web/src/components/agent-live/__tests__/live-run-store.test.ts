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

import { describe, expect, it } from 'vitest';

import { LiveRunStore, mergeBySourceSeq, timelineKey } from '../live-run-store';

import type { PublicWebEvent } from '@/adapters/rara-stream';
import type { TimelineItem } from '@/api/kernel-types';

const startEvent = { type: '__stream_started' } satisfies PublicWebEvent;
const closeEvent = { type: '__stream_closed' } satisfies PublicWebEvent;

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
