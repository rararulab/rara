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
 * React hook that subscribes to the cross-session topology WebSocket
 * (`/api/v1/kernel/chat/topology/{root_session_key}`) and returns a
 * de-duplicated buffer of `WebEvent`s observed on the root session and
 * all transitively spawned descendants.
 *
 * Wire format mirrors `TopologyFrame` in
 * `crates/channels/src/web_topology.rs` — `hello`, `session_subscribed`,
 * `event`, `error`. Unknown variants are ignored so additive backend
 * changes do not break old clients (same policy as `session-ws-client`).
 *
 * Reconnect uses the same bounded exponential backoff schedule as
 * `SessionWsClient` so the topology endpoint behaves like the per-session
 * one. The schedule is a mechanism constant, not config — see
 * `docs/guides/anti-patterns.md` ("mechanism constants are not config").
 */

import { useEffect, useRef, useState } from 'react';

import { buildWsBaseUrl } from '@/adapters/ws-base-url';
import type { WebFrame } from '@/agent/session-ws-client';
import { getAccessToken } from '@/api/client';

// ---------------------------------------------------------------------------
// Wire frames — server → client (topology endpoint)
//
// Mirrors `TopologyFrame` in `crates/channels/src/web_topology.rs`. Kept
// local to this hook on purpose: the existing `session-ws-client.ts` owns
// the per-session frame union and we don't want the topology variants to
// leak into that client until tasks #6/#8 collapse the two endpoints.
//
// `TopologyWebFrame` extends `WebFrame` with the three multi-agent
// topology variants the backend forwards through the topology socket
// (`SubagentSpawned` / `SubagentDone` / `TapeForked` — see
// `crates/channels/src/web.rs`). They are intentionally NOT added to
// `WebFrame` itself: the persistent per-session WS already passes them
// through, but `RaraAgent` doesn't consume them yet, and tasks #6/#8
// will collapse the two endpoints into a single client.
// ---------------------------------------------------------------------------

export type TopologyWebFrame =
  | WebFrame
  | {
      type: 'subagent_spawned';
      parent_session: string;
      child_session: string;
      manifest_name: string;
    }
  | {
      type: 'subagent_done';
      parent_session: string;
      child_session: string;
      success: boolean;
    }
  | {
      type: 'tape_forked';
      parent_session: string;
      forked_from: string;
      child_tape: string;
      forked_at_anchor?: string | null;
    };

/** Descendant entry in the topology `hello` snapshot. */
export interface TopologyDescendant {
  session_key: string;
  parent: string;
}

/** Discriminated union of frames received from the topology WS. */
export type TopologyFrame =
  | {
      type: 'hello';
      root_session_key: string;
      initial_descendants: TopologyDescendant[];
    }
  | {
      type: 'session_subscribed';
      session_key: string;
      parent: string | null;
    }
  | {
      type: 'event';
      session_key: string;
      event: TopologyWebFrame;
    }
  | { type: 'error'; message: string };

/**
 * One observed event tagged with its originating session, in arrival
 * order. The buffer is the concatenation of every `event` frame the
 * socket has received this connection (it resets on reconnect, mirroring
 * how the legacy chat WS handled stream restarts).
 */
export interface TopologyEventEntry {
  /** Monotonically increasing per-connection sequence number. */
  seq: number;
  /** Session that emitted the underlying `StreamEvent`. */
  sessionKey: string;
  /** Mapped `WebEvent` payload. */
  event: TopologyWebFrame;
}

/**
 * Connection lifecycle states surfaced to the UI. Matches the spirit of
 * `SessionWsClient`'s `LifecycleEvent` but flattened to a single status
 * because consumers only need to render a status pill.
 */
export type TopologyStatus =
  | { kind: 'idle' }
  | { kind: 'connecting' }
  | { kind: 'open' }
  | { kind: 'reconnecting'; attempt: number; delayMs: number }
  | { kind: 'closed'; reason: 'auth' | 'reconnect_exhausted' | 'no_session' };

/** Reactive state returned by {@link useTopologySubscription}. */
export interface TopologySubscription {
  /** The root session key currently being subscribed to (echo of input). */
  rootSessionKey: string | null;
  /** Connection status for the topology socket. */
  status: TopologyStatus;
  /**
   * Map of every session known to the connection — the root plus every
   * descendant announced via `session_subscribed`. Value is the parent
   * session key (or `null` for the root).
   */
  sessions: Map<string, string | null>;
  /** Every observed `event` frame in arrival order. */
  events: TopologyEventEntry[];
}

// ---------------------------------------------------------------------------
// Reconnect tuning — mechanism constants, not config. Aligned with
// `session-ws-client.ts` so both WS endpoints behave identically.
// ---------------------------------------------------------------------------

const RECONNECT_BACKOFF_MS = [250, 500, 1_000, 2_000, 4_000] as const;
const RECONNECT_BACKOFF_CAP_MS = 5_000;
const MAX_RECONNECT_ATTEMPTS = RECONNECT_BACKOFF_MS.length;

/**
 * Subscribe to the cross-session topology WebSocket for `rootSessionKey`.
 *
 * Pass `null` to disable the subscription (e.g. before the user has
 * picked a root). The hook owns one socket per non-null key; switching
 * keys tears down the current socket and opens a new one.
 *
 * The returned state is a single `useState` value so consumers can pass
 * it through React.memo without hitting referential equality surprises
 * on independent fields.
 */
export function useTopologySubscription(rootSessionKey: string | null): TopologySubscription {
  const [state, setState] = useState<TopologySubscription>(() => ({
    rootSessionKey,
    status: { kind: 'idle' },
    sessions: new Map(),
    events: [],
  }));

  // The latest "live" buffer the socket appends to. Kept in a ref so the
  // onmessage closure does not capture a stale `events` array between
  // renders — we batch React state updates via `setState` but mutate the
  // ref synchronously for correct ordering across rapid frames.
  const seqRef = useRef(0);

  useEffect(() => {
    if (!rootSessionKey) {
      setState({
        rootSessionKey: null,
        status: { kind: 'closed', reason: 'no_session' },
        sessions: new Map(),
        events: [],
      });
      return;
    }

    let disposed = false;
    let socket: WebSocket | null = null;
    let reconnectAttempts = 0;
    let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
    seqRef.current = 0;

    setState({
      rootSessionKey,
      status: { kind: 'connecting' },
      sessions: new Map(),
      events: [],
    });

    const buildUrl = (): string | null => {
      const token = getAccessToken();
      // Browser WebSocket cannot set headers, so the query token is
      // mandatory. Without it the backend will 401, so bail early.
      if (!token) return null;
      const base = buildWsBaseUrl();
      const path = `/api/v1/kernel/chat/topology/${encodeURIComponent(rootSessionKey)}`;
      const params = new URLSearchParams({ token });
      return `${base}${path}?${params.toString()}`;
    };

    const handleFrame = (frame: TopologyFrame): void => {
      switch (frame.type) {
        case 'hello': {
          // `hello` is the proof-of-life signal — backend can early-close
          // after `onopen` if state is missing. Reset retry budget here.
          reconnectAttempts = 0;
          const sessions = new Map<string, string | null>();
          sessions.set(frame.root_session_key, null);
          for (const d of frame.initial_descendants) {
            sessions.set(d.session_key, d.parent);
          }
          setState((prev) => ({
            ...prev,
            status: { kind: 'open' },
            sessions,
            events: [],
          }));
          return;
        }
        case 'session_subscribed': {
          setState((prev) => {
            const next = new Map(prev.sessions);
            next.set(frame.session_key, frame.parent);
            return { ...prev, sessions: next };
          });
          return;
        }
        case 'event': {
          seqRef.current += 1;
          const entry: TopologyEventEntry = {
            seq: seqRef.current,
            sessionKey: frame.session_key,
            event: frame.event,
          };
          setState((prev) => ({ ...prev, events: [...prev.events, entry] }));
          return;
        }
        case 'error': {
          console.warn('topology WS error frame', frame.message);
          return;
        }
        default:
          // Unknown variant — additive backend changes are tolerated.
          return;
      }
    };

    const scheduleReconnect = (): void => {
      if (disposed) return;
      if (reconnectAttempts >= MAX_RECONNECT_ATTEMPTS) {
        disposed = true;
        setState((prev) => ({
          ...prev,
          status: { kind: 'closed', reason: 'reconnect_exhausted' },
        }));
        return;
      }
      const delayMs = RECONNECT_BACKOFF_MS[reconnectAttempts] ?? RECONNECT_BACKOFF_CAP_MS;
      const attempt = reconnectAttempts + 1;
      reconnectAttempts = attempt;
      setState((prev) => ({
        ...prev,
        status: { kind: 'reconnecting', attempt, delayMs },
      }));
      if (reconnectTimer !== null) clearTimeout(reconnectTimer);
      reconnectTimer = setTimeout(() => {
        reconnectTimer = null;
        if (disposed) return;
        openSocket();
      }, delayMs);
    };

    const openSocket = (): void => {
      const url = buildUrl();
      if (!url) {
        disposed = true;
        setState((prev) => ({ ...prev, status: { kind: 'closed', reason: 'auth' } }));
        return;
      }
      let ws: WebSocket;
      try {
        ws = new WebSocket(url);
      } catch (err) {
        console.warn('topology WS constructor threw', err);
        scheduleReconnect();
        return;
      }
      socket = ws;

      ws.onmessage = (ev: MessageEvent) => {
        let frame: TopologyFrame;
        try {
          frame = JSON.parse(ev.data as string) as TopologyFrame;
        } catch {
          return;
        }
        handleFrame(frame);
      };

      ws.onerror = () => {};

      ws.onclose = () => {
        if (ws !== socket) return;
        socket = null;
        if (disposed) return;
        scheduleReconnect();
      };
    };

    openSocket();

    return () => {
      disposed = true;
      if (reconnectTimer !== null) {
        clearTimeout(reconnectTimer);
        reconnectTimer = null;
      }
      const ws = socket;
      socket = null;
      if (ws) {
        ws.onopen = null;
        ws.onmessage = null;
        ws.onerror = null;
        ws.onclose = null;
        try {
          ws.close();
        } catch {
          // already closed
        }
      }
    };
  }, [rootSessionKey]);

  return state;
}
