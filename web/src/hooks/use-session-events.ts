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

import { useEffect, useRef } from 'react';

import { getAccessToken } from '@/api/client';

/**
 * Frame contract mirrored from `rara-channels::web_session_events::SessionEventFrame`.
 * Stable additive: clients ignore unknown `type` values.
 */
export type SessionEventFrame =
  | { type: 'hello' }
  | {
      type: 'tape_appended';
      entry_id: number;
      role: string | null;
      timestamp: string;
    };

export interface UseSessionEventsOptions {
  /** Active session key. When `null`, the hook closes any open WS. */
  sessionKey: string | null;
  /** Called for each `tape_appended` frame; the caller decides how to refresh state. */
  onTapeAppended: (event: { entry_id: number; role: string | null; timestamp: string }) => void;
}

// Mechanism-tuning constants — internal knobs, not deploy-relevant config.
// Aligned with `rara-stream.ts`: bounded retry budget so a permanently dead
// backend cannot spin reconnect attempts forever.
const RECONNECT_BACKOFF_MS = [250, 500, 1_000, 2_000, 4_000] as const;
const MAX_RECONNECT_ATTEMPTS = RECONNECT_BACKOFF_MS.length;

/**
 * Maintain a persistent WebSocket subscription to the kernel's session
 * event bus so the UI sees tape mutations that arrive outside a live
 * user turn (background-task summaries, scheduled re-entries, …).
 *
 * Reconnects with a bounded retry budget ({@link MAX_RECONNECT_ATTEMPTS}).
 * The backend may close a socket immediately after `onopen` (before the
 * `Hello` frame) when the kernel handle is not yet attached, so the retry
 * counter is reset only on receipt of a `Hello` frame — `onopen` alone is
 * not proof of a live connection. Lifecycle is tied to `sessionKey`:
 * switching sessions closes the old socket and opens a new one; setting
 * `sessionKey` to `null` closes the socket cleanly.
 */
export function useSessionEvents({ sessionKey, onTapeAppended }: UseSessionEventsOptions): void {
  // Stable ref so the effect does not re-subscribe when callers pass a
  // fresh closure each render. Updated inside an effect to satisfy the
  // "no ref writes during render" lint.
  const onTapeAppendedRef = useRef(onTapeAppended);
  useEffect(() => {
    onTapeAppendedRef.current = onTapeAppended;
  }, [onTapeAppended]);

  useEffect(() => {
    if (!sessionKey) return;

    let attempts = 0;
    let cancelled = false;
    let ws: WebSocket | null = null;
    let retryTimer: ReturnType<typeof setTimeout> | null = null;

    const connect = () => {
      if (cancelled) return;
      const token = getAccessToken();
      if (!token) return;

      const host = window.location.host;
      const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
      const url = `${protocol}//${host}/api/v1/kernel/chat/events/${encodeURIComponent(
        sessionKey,
      )}?token=${encodeURIComponent(token)}`;

      ws = new WebSocket(url);

      ws.onmessage = (ev) => {
        let frame: SessionEventFrame;
        try {
          frame = JSON.parse(ev.data) as SessionEventFrame;
        } catch {
          return;
        }
        if (frame.type === 'hello') {
          // Hello is the only signal the connection is truly alive — backend
          // can early-close after `onopen` if the kernel handle is missing.
          attempts = 0;
          return;
        }
        if (frame.type === 'tape_appended') {
          onTapeAppendedRef.current({
            entry_id: frame.entry_id,
            role: frame.role,
            timestamp: frame.timestamp,
          });
        }
      };

      ws.onerror = () => {
        // Let `onclose` drive reconnect — `onerror` always precedes it.
      };

      ws.onclose = () => {
        ws = null;
        if (cancelled) return;
        if (attempts >= MAX_RECONNECT_ATTEMPTS) {
          console.warn(
            `[useSessionEvents] giving up after ${MAX_RECONNECT_ATTEMPTS} reconnect attempts for session ${sessionKey}`,
          );
          return;
        }
        const delay = RECONNECT_BACKOFF_MS[attempts];
        attempts += 1;
        retryTimer = setTimeout(connect, delay);
      };
    };

    connect();

    return () => {
      cancelled = true;
      if (retryTimer) clearTimeout(retryTimer);
      if (ws) {
        // Detach handlers so an in-flight close does not schedule a retry
        // after the effect has unmounted.
        ws.onopen = null;
        ws.onmessage = null;
        ws.onerror = null;
        ws.onclose = null;
        try {
          ws.close();
        } catch {
          /* ignore */
        }
      }
    };
  }, [sessionKey]);
}
