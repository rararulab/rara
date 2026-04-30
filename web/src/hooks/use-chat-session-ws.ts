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
 * React wrapper around `SessionWsClient` for the topology prompt editor.
 *
 * The editor needs only three things from the per-session WS:
 *
 * 1. `sendPrompt(content)` — push a new user turn.
 * 2. `sendAbort()`         — interrupt the in-flight stream.
 * 3. A connection / streaming status flag so the Send button can
 *    morph into Stop and the textarea can disable on disconnect.
 *
 * Topology events themselves are already streamed via
 * `useTopologySubscription`; subscribing here too would double-render
 * every frame. So this hook owns the SOCKET but exposes no event buffer.
 *
 * One socket per `sessionKey`. Switching keys disconnects the old socket
 * and opens a new one — same lifecycle as `SessionPicker` selection.
 */

import { useCallback, useEffect, useRef, useState } from 'react';

import {
  SessionWsClient,
  type LifecycleEvent,
  type PromptContent,
  type WebFrame,
} from '@/agent/session-ws-client';

/** UI-facing connection status. Collapsed from `LifecycleEvent` because
 *  the editor only needs three states: input enabled, input enabled but
 *  Stop visible, input disabled. */
export type ChatSessionWsStatus =
  | 'idle' // no sessionKey provided yet
  | 'connecting' // socket open but no `hello` yet
  | 'live' // `hello` received, no in-flight turn
  | 'streaming' // backend is currently producing deltas for our last prompt
  | 'reconnecting' // dropped, waiting on retry
  | 'closed'; // user closed or retry exhausted / auth failed

export interface ChatSessionWs {
  status: ChatSessionWsStatus;
  /** Last error surfaced from the backend (`error` frame) or auth failure.
   *  Cleared on the next successful `hello`. */
  error: string | null;
  /** Push a `prompt` frame. Returns `false` if the socket is not open. */
  sendPrompt: (content: PromptContent) => boolean;
  /** Push an `abort` frame. Returns `false` if the socket is not open. */
  sendAbort: () => boolean;
}

/**
 * Manage one per-session WebSocket on behalf of the editor. Pass `null`
 * to drop the socket (e.g. before the user has selected a session).
 */
export function useChatSessionWs(sessionKey: string | null): ChatSessionWs {
  const [status, setStatus] = useState<ChatSessionWsStatus>(sessionKey ? 'connecting' : 'idle');
  const [error, setError] = useState<string | null>(null);
  // Hold the live client in a ref so the send/abort callbacks have a
  // stable identity across renders without re-running the connect effect.
  const clientRef = useRef<SessionWsClient | null>(null);

  useEffect(() => {
    if (!sessionKey) {
      setStatus('idle');
      setError(null);
      clientRef.current = null;
      return;
    }

    setStatus('connecting');
    setError(null);

    const client = new SessionWsClient({ sessionKey });
    clientRef.current = client;

    const offFrame = client.onFrame((frame: WebFrame) => {
      switch (frame.type) {
        case 'typing':
          setStatus('streaming');
          return;
        case 'done':
          setStatus('live');
          return;
        case 'error':
          // Backend `error` frames don't terminate the socket — surface
          // the message but keep the wire status as-is so the user can
          // retry without reconnect.
          setError(frame.message);
          setStatus('live');
          return;
        default:
          return;
      }
    });

    const offLifecycle = client.onLifecycle((event: LifecycleEvent) => {
      switch (event.type) {
        case 'connected':
          setStatus('live');
          setError(null);
          return;
        case 'reconnecting':
          setStatus('reconnecting');
          return;
        case 'closed':
          setStatus('closed');
          if (event.reason === 'auth') {
            setError('authentication failed — please log in again');
          } else if (event.reason === 'reconnect_exhausted') {
            setError('connection lost — refresh to retry');
          }
          return;
      }
    });

    client.connect();

    return () => {
      offFrame();
      offLifecycle();
      client.disconnect();
      if (clientRef.current === client) {
        clientRef.current = null;
      }
    };
  }, [sessionKey]);

  const sendPrompt = useCallback((content: PromptContent): boolean => {
    const client = clientRef.current;
    if (!client) return false;
    const ok = client.prompt(content);
    if (ok) {
      // Optimistically flip to streaming so the Send button morphs to
      // Stop immediately; the backend `typing` frame will confirm.
      setStatus('streaming');
      setError(null);
    }
    return ok;
  }, []);

  const sendAbort = useCallback((): boolean => {
    const client = clientRef.current;
    if (!client) return false;
    return client.abort();
  }, []);

  return { status, error, sendPrompt, sendAbort };
}
