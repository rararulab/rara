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
 * Live-run store for the AgentLiveCard.
 *
 * Receives raw {@link PublicWebEvent} frames from the rara-stream adapter
 * (one observer per session), projects them into normalized
 * {@link TimelineItem} rows, and groups consecutive events into a
 * {@link LiveRun}. At most one `active` run exists per session key at any
 * time — subsequent synthetic `__stream_started` frames push the prior
 * run into {@link TaskRunHistoryEntry}.
 *
 * The store is deliberately vanilla (no React dependency) so the same
 * instance can be consumed by `useSyncExternalStore` from multiple
 * component trees, and unit-tested without a DOM.
 */

import type { PublicWebEvent } from '@/adapters/rara-stream';
import type { TimelineItem } from '@/api/kernel-types';

/** Status of a single agent run. */
export type RunStatus = 'running' | 'completed' | 'failed' | 'cancelled';

/** One agent run — either currently active or moved into history. */
export interface LiveRun {
  /** Stable id across the run's lifetime (`{sessionKey}-{counter}`). */
  runId: string;
  sessionKey: string;
  status: RunStatus;
  /** `Date.now()` when the run was opened. */
  startedAt: number;
  /** `Date.now()` at completion (null while running). */
  endedAt: number | null;
  /** Normalized timeline events, keyed for React by `{source}:{seq}`. */
  items: TimelineItem[];
  /** Number of distinct `tool_use` events recorded in this run. */
  toolCalls: number;
  /** Last error message (for `failed` status); null otherwise. */
  error: string | null;
}

/** Snapshot of the store's per-session slice. */
export interface SessionSlice {
  active: LiveRun | null;
  history: LiveRun[];
}

const EMPTY_SLICE: SessionSlice = { active: null, history: [] };

/** Listener callback fired after a mutation. */
type Listener = () => void;

// The "live" source label used for seq keys. Historical-vs-live dedup
// keys combine `source + seq` so the two spaces can coexist in the same
// TimelineItem array without collisions (see kernel-types.ts lines 257-258).
const LIVE_SOURCE = 'l';

/** Combined React reconciliation key for a timeline row. */
export function timelineKey(source: string, seq: number): string {
  return `${source}-${seq}`;
}

/** Build a deterministic runId counter so snapshots are stable in tests. */
let runCounter = 0;
function nextRunId(sessionKey: string): string {
  runCounter += 1;
  return `${sessionKey}-${runCounter}`;
}

/**
 * Tiny vanilla store. External API is deliberately narrow:
 *
 * - `publish(sessionKey, event)` — feed a raw WebEvent.
 * - `subscribe(sessionKey, listener)` — observe mutations for a session.
 * - `snapshot(sessionKey)` — read the current slice.
 */
export class LiveRunStore {
  private readonly slices = new Map<string, SessionSlice>();
  private readonly listeners = new Map<string, Set<Listener>>();

  /** Read the current immutable slice for a session. */
  snapshot(sessionKey: string): SessionSlice {
    return this.slices.get(sessionKey) ?? EMPTY_SLICE;
  }

  /** Subscribe to mutations; returns an unsubscribe fn. */
  subscribe(sessionKey: string, listener: Listener): () => void {
    let set = this.listeners.get(sessionKey);
    if (!set) {
      set = new Set();
      this.listeners.set(sessionKey, set);
    }
    set.add(listener);
    return () => {
      set?.delete(listener);
    };
  }

  /** Feed a raw WebEvent frame. */
  publish(sessionKey: string, event: PublicWebEvent): void {
    const current = this.slices.get(sessionKey) ?? { active: null, history: [] };
    const next = reduce(current, sessionKey, event);
    if (next === current) return;
    this.slices.set(sessionKey, next);
    this.emit(sessionKey);
  }

  /** Clear a session's state — called on session switch / unmount. */
  reset(sessionKey: string): void {
    if (!this.slices.has(sessionKey)) return;
    this.slices.delete(sessionKey);
    this.emit(sessionKey);
  }

  private emit(sessionKey: string): void {
    const set = this.listeners.get(sessionKey);
    if (!set) return;
    for (const listener of set) listener();
  }
}

// ---------------------------------------------------------------------------
// Reducer — pure, exported for unit tests.
// ---------------------------------------------------------------------------

/**
 * Apply one WebEvent to the slice, returning a new immutable slice.
 *
 * The reducer is the only place seq assignment and dedup happen, so
 * every mutation path flows through the same invariants:
 *   1. Seqs are monotonically allocated per run.
 *   2. Each item's React key is `{source}-{seq}` — two events with the
 *      same key are treated as a dedup (idempotent replay).
 *   3. Tool-result events resolve back to their `tool_use` twin by
 *      backend `id`; the pair gets distinct seqs but shares a `tool`.
 */
export function reduce(
  slice: SessionSlice,
  sessionKey: string,
  event: PublicWebEvent,
): SessionSlice {
  const type = event.type;

  if (type === '__stream_started') {
    // Retire any run that was left dangling (e.g. WS reconnect) before
    // opening a new active run. `cancelled` rather than `completed` so
    // history UI reads accurately.
    const retired = slice.active ? finalize(slice.active, 'cancelled', 'Stream restarted') : null;
    const history = retired ? [retired, ...slice.history] : slice.history;
    const active: LiveRun = {
      runId: nextRunId(sessionKey),
      sessionKey,
      status: 'running',
      startedAt: Date.now(),
      endedAt: null,
      items: [],
      toolCalls: 0,
      error: null,
    };
    return { active, history };
  }

  if (type === '__stream_closed') {
    if (!slice.active) return slice;
    // Close the run if no terminal `done`/`error` has arrived yet — the
    // WebSocket hung up mid-flight, treat as cancelled.
    if (slice.active.status !== 'running') {
      return { active: null, history: [slice.active, ...slice.history] };
    }
    const retired = finalize(slice.active, 'cancelled', 'Stream closed');
    return { active: null, history: [retired, ...slice.history] };
  }

  // All remaining events need an active run.
  if (!slice.active) return slice;
  const run = slice.active;

  switch (type) {
    case 'done': {
      const retired = finalize(run, 'completed', null);
      return { active: null, history: [retired, ...slice.history] };
    }
    case 'error': {
      const message = readString(event, 'message') ?? 'Unknown error';
      const retired = finalize({ ...run, error: message }, 'failed', message);
      return { active: null, history: [retired, ...slice.history] };
    }
    case 'reasoning_delta': {
      const text = readString(event, 'text') ?? '';
      if (!text) return slice;
      return { ...slice, active: appendOrMergeDelta(run, 'thinking', text) };
    }
    case 'text_delta': {
      const text = readString(event, 'text') ?? '';
      if (!text) return slice;
      return { ...slice, active: appendOrMergeDelta(run, 'agent', text) };
    }
    case 'tool_call_start': {
      const name = readString(event, 'name') ?? 'tool';
      const id = readString(event, 'id') ?? '';
      const args = readRecord(event, 'arguments');
      // Dedup by `{id}:start` — if we have already recorded this start,
      // the event is a replay.
      if (id && run.items.some((it) => it.kind === 'tool_use' && it.tool && idTag(it) === id)) {
        return slice;
      }
      const item: TimelineItem = {
        seq: nextSeq(run),
        turn: 0,
        kind: 'tool_use',
        tool: name,
        input: args ?? {},
        streaming: true,
      };
      tagId(item, id);
      return {
        ...slice,
        active: {
          ...run,
          items: [...run.items, item],
          toolCalls: run.toolCalls + 1,
        },
      };
    }
    case 'tool_call_end': {
      const id = readString(event, 'id') ?? '';
      const preview = readString(event, 'result_preview') ?? '';
      const error = readString(event, 'error');
      const success = readBool(event, 'success') ?? true;
      // Resolve the matching `tool_use` to clear the streaming flag.
      const nextItems = run.items.map((it) => {
        if (it.kind === 'tool_use' && idTag(it) === id) {
          return { ...it, streaming: false, success };
        }
        return it;
      });
      // Append a tool_result (or error row when the backend signals one).
      const toolName = run.items.find((it) => it.kind === 'tool_use' && idTag(it) === id)?.tool;
      if (error) {
        nextItems.push({
          seq: nextSeq(run),
          turn: 0,
          kind: 'error',
          ...(toolName === undefined ? {} : { tool: toolName }),
          content: error,
        });
      } else {
        nextItems.push({
          seq: nextSeq(run),
          turn: 0,
          kind: 'tool_result',
          ...(toolName === undefined ? {} : { tool: toolName }),
          output: preview,
          success,
        });
      }
      return { ...slice, active: { ...run, items: nextItems } };
    }
    default:
      // Everything else is informational and ignored for the live card.
      return slice;
  }
}

/** Freeze a run by stamping `endedAt` + status. */
function finalize(run: LiveRun, status: RunStatus, error: string | null): LiveRun {
  return {
    ...run,
    status,
    endedAt: Date.now(),
    items: run.items.map((it) => ({ ...it, streaming: false })),
    error,
  };
}

/** Allocate the next seq within a run (monotonic). */
function nextSeq(run: LiveRun): number {
  return run.items.length === 0
    ? 0
    : (run.items[run.items.length - 1]?.seq ?? run.items.length - 1) + 1;
}

/**
 * Append a streaming text delta, merging into the last item if it
 * matches the same kind (so thinking/agent deltas accumulate into one
 * row instead of exploding into N-character rows).
 */
function appendOrMergeDelta(run: LiveRun, kind: 'thinking' | 'agent', text: string): LiveRun {
  const last = run.items[run.items.length - 1];
  if (last && last.kind === kind && last.streaming) {
    const merged: TimelineItem = {
      ...last,
      content: (last.content ?? '') + text,
    };
    const items = run.items.slice(0, -1);
    items.push(merged);
    return { ...run, items };
  }
  const item: TimelineItem = {
    seq: nextSeq(run),
    turn: 0,
    kind,
    content: text,
    streaming: true,
  };
  return { ...run, items: [...run.items, item] };
}

function readString(obj: object, key: string): string | null {
  const v = (obj as Record<string, unknown>)[key];
  return typeof v === 'string' ? v : null;
}

function readBool(obj: object, key: string): boolean | null {
  const v = (obj as Record<string, unknown>)[key];
  return typeof v === 'boolean' ? v : null;
}

function readRecord(obj: object, key: string): Record<string, unknown> | null {
  const v = (obj as Record<string, unknown>)[key];
  return v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : null;
}

// ---------------------------------------------------------------------------
// Per-tool-call backend id tracking
// ---------------------------------------------------------------------------
// TimelineItem intentionally does not carry the backend `tool_call_id`
// (see kernel-types.ts). For live-run correlation we tag items via a
// WeakMap so the id never leaks into the public TimelineItem surface.

const ID_TAGS = new WeakMap<TimelineItem, string>();

function tagId(item: TimelineItem, id: string): void {
  if (id) ID_TAGS.set(item, id);
}

function idTag(item: TimelineItem): string | undefined {
  return ID_TAGS.get(item);
}

// ---------------------------------------------------------------------------
// Shared instance + dedup helper for interleaved historical + live sources
// ---------------------------------------------------------------------------

/** Process-wide singleton used by PiChat. */
export const liveRunStore = new LiveRunStore();

/**
 * Merge historical and live items into one array, deduped by
 * `{source}:{seq}` key. Ordering is preserved from each input (both are
 * already monotonic within their own source).
 */
export function mergeBySourceSeq(historical: TimelineItem[], live: TimelineItem[]): TimelineItem[] {
  const seen = new Set<string>();
  const out: TimelineItem[] = [];
  for (const it of historical) {
    const key = timelineKey('h', it.seq);
    if (!seen.has(key)) {
      seen.add(key);
      out.push(it);
    }
  }
  for (const it of live) {
    const key = timelineKey(LIVE_SOURCE, it.seq);
    if (!seen.has(key)) {
      seen.add(key);
      out.push(it);
    }
  }
  return out;
}

export { LIVE_SOURCE };
