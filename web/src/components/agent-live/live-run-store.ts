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
  /**
   * Latest `progress.stage` string emitted by the kernel — a free-text
   * status marker (e.g. `"thinking"`, `"Waiting for LLM response
   * (iteration 2)..."`, `"Processing... (3 steps completed)"`). Rendered
   * in the live card body when the run has no substantive timeline items
   * yet — otherwise the card would show an unhelpful "no data" placeholder
   * while the provider is still producing its first chunk.
   */
  currentStage: string | null;
}

/**
 * Snapshot of the store's per-session slice.
 *
 * Fields are `readonly` so consumers cannot accidentally mutate the
 * store's internal state (e.g. `slice.history.push(...)` would be a
 * type error). The reducer always produces a brand new slice via
 * spread, so this costs nothing at runtime.
 */
export interface SessionSlice {
  readonly active: LiveRun | null;
  readonly history: readonly LiveRun[];
}

/**
 * Shared frozen empty slice.
 *
 * Exported so `useLiveRun` can return the same referentially stable
 * value when `sessionKey` is undefined — `useSyncExternalStore` relies
 * on snapshot identity to bail out of re-renders (React error #185
 * otherwise). Both the outer object and the inner array are frozen so
 * runtime mutation throws in strict mode.
 */
export const EMPTY_SLICE: SessionSlice = Object.freeze({
  active: null,
  history: Object.freeze([] as LiveRun[]),
});

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

/**
 * Delay between a run reaching a terminal state and the live card
 * being auto-retired to history. Lives at module scope so tests and
 * the SingleAgentLiveCard fade-out can read the same value.
 */
export const AUTO_DISMISS_MS = 5_000;

/**
 * Factory for a per-store run-id generator. Counter lives inside each
 * store instance so parallel test stores don't share global state (and
 * neither does the production singleton leak into tests that construct
 * their own `new LiveRunStore()`).
 */
function makeRunIdFactory(): (sessionKey: string) => string {
  let counter = 0;
  return (sessionKey) => {
    counter += 1;
    return `${sessionKey}-${counter}`;
  };
}

/**
 * Default run-id generator for ad-hoc `reduce` calls (e.g. unit tests
 * that exercise the pure reducer directly without constructing a
 * store). Production code always goes through `LiveRunStore`, which
 * owns its own factory.
 */
const defaultRunId = makeRunIdFactory();

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
  private readonly nextRunId = makeRunIdFactory();
  // Pending auto-dismiss timers, keyed by sessionKey. A timer is armed
  // when a publish transitions the active run from `running` to a
  // terminal state and is cleared on retire / reset / new stream start.
  private readonly dismissTimers = new Map<string, ReturnType<typeof setTimeout>>();

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
    const current = this.slices.get(sessionKey) ?? EMPTY_SLICE;
    const next = reduce(current, sessionKey, event, this.nextRunId);
    if (next === current) return;
    this.slices.set(sessionKey, next);
    this.scheduleAutoDismiss(sessionKey, current.active, next.active);
    this.emit(sessionKey);
  }

  /** Clear a session's state — called on session switch / unmount. */
  reset(sessionKey: string): void {
    this.cancelAutoDismiss(sessionKey);
    if (!this.slices.has(sessionKey)) return;
    this.slices.delete(sessionKey);
    this.emit(sessionKey);
  }

  /**
   * Arm or cancel the auto-dismiss timer for a session based on the
   * active-run transition produced by the latest publish. The timer is
   * an effect, not a reducer concern — keeping it here preserves the
   * reducer's purity while still guaranteeing exactly one pending
   * timer per session.
   */
  private scheduleAutoDismiss(
    sessionKey: string,
    prev: LiveRun | null,
    next: LiveRun | null,
  ): void {
    // If the active run is gone (retired) or is running again, drop any
    // pending dismissal — there is nothing terminal to clear.
    if (!next || next.status === 'running') {
      this.cancelAutoDismiss(sessionKey);
      return;
    }
    // Already terminal across both snapshots and same run — keep the
    // existing timer running rather than restarting it on every event.
    if (prev && prev.runId === next.runId && prev.status === next.status) {
      return;
    }
    this.cancelAutoDismiss(sessionKey);
    const timer = setTimeout(() => {
      this.dismissTimers.delete(sessionKey);
      this.dismissActive(sessionKey, next.runId);
    }, AUTO_DISMISS_MS);
    this.dismissTimers.set(sessionKey, timer);
  }

  private cancelAutoDismiss(sessionKey: string): void {
    const timer = this.dismissTimers.get(sessionKey);
    if (timer === undefined) return;
    clearTimeout(timer);
    this.dismissTimers.delete(sessionKey);
  }

  /**
   * Retire the active run to history — invoked by the auto-dismiss
   * timer. Guarded by `runId` so a stale timer racing against a new
   * stream cannot evict a freshly-started run.
   */
  private dismissActive(sessionKey: string, runId: string): void {
    const slice = this.slices.get(sessionKey);
    if (!slice || !slice.active || slice.active.runId !== runId) return;
    const next: SessionSlice = {
      active: null,
      history: [slice.active, ...slice.history],
    };
    this.slices.set(sessionKey, next);
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
  nextRunId: (sessionKey: string) => string = defaultRunId,
): SessionSlice {
  const type = event.type;

  if (type === '__stream_started') {
    // Retire the previous active run before opening a new one. If the run
    // already terminated (`completed` / `failed`), keep its recorded
    // status; only dangling `running` runs flip to `cancelled`. The
    // reducer keeps terminal runs in the `active` slot until the next
    // stream starts so the UI can render a persistent "last completed"
    // card instead of an abrupt unmount on `done`.
    const prev = slice.active;
    const retired = prev
      ? prev.status === 'running'
        ? finalize(prev, 'cancelled', 'Stream restarted')
        : prev
      : null;
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
      currentStage: null,
    };
    return { active, history };
  }

  if (type === '__stream_closed') {
    if (!slice.active) return slice;
    // Already terminal (done/error arrived before close) — nothing to do;
    // the card stays pinned in the active slot until the next run.
    if (slice.active.status !== 'running') {
      return slice;
    }
    // WebSocket hung up mid-flight — mark cancelled but keep visible so
    // the viewer can inspect what ran before the drop.
    return {
      ...slice,
      active: finalize(slice.active, 'cancelled', 'Stream closed'),
    };
  }

  // All remaining events need an active run.
  if (!slice.active) return slice;
  const run = slice.active;

  switch (type) {
    case 'done': {
      // Keep the run in the active slot so the card persists as a
      // "last completed" summary until the next stream starts. History
      // still gets populated when a new run replaces this one.
      return { ...slice, active: finalize(run, 'completed', null) };
    }
    case 'error': {
      const message = readString(event, 'message') ?? 'Unknown error';
      return {
        ...slice,
        active: finalize({ ...run, error: message }, 'failed', message),
      };
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
    case 'progress': {
      // Free-text status marker from the kernel (see
      // `crates/kernel/src/agent/mod.rs` emit sites). Mirrored onto the
      // run so the card header can read the latest stage even when no
      // timeline items have landed yet — common for LLM providers that
      // don't stream text deltas (e.g. MiniMax batch mode).
      const stage = readString(event, 'stage');
      if (!stage) return slice;
      return { ...slice, active: { ...run, currentStage: stage } };
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
//
// Lifecycle: tags are only meaningful while a `tool_use` is still
// running — `finalize()` and `tool_call_end` both create new item
// objects via `{ ...it, ... }`, which intentionally drops the tag on
// the replacement. That is fine because the id is only needed to pair
// a `tool_call_end` with its still-streaming `tool_use`; once the pair
// resolves, the tag serves no purpose and the old entry is eligible
// for GC together with the old item reference.

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
