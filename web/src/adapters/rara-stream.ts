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

import {
  BASE_URL,
  getAccessToken,
  getAuthUser,
  getBackendUrl,
  redirectToLogin,
} from '@/api/client';

// ---------------------------------------------------------------------------
// WebEvent — frames received from the rara WebSocket chat API
// ---------------------------------------------------------------------------

/** Discriminated union of all WebSocket event types from the rara backend. */
type WebEvent =
  | { type: 'text_delta'; text: string }
  | { type: 'reasoning_delta'; text: string }
  | { type: 'typing' }
  | {
      type: 'tool_call_start';
      name: string;
      id: string;
      arguments: Record<string, unknown>;
    }
  | {
      type: 'tool_call_end';
      id: string;
      result_preview: string;
      success: boolean;
      error: string | null;
    }
  | { type: 'progress'; stage: string }
  | { type: 'done' }
  | { type: 'message'; content: string }
  | { type: 'error'; message: string }
  | { type: 'turn_rationale'; text: string }
  | {
      type: 'turn_metrics';
      duration_ms: number;
      iterations: number;
      tool_calls: number;
      model: string;
    }
  | {
      type: 'usage';
      input: number;
      output: number;
      cache_read: number;
      cache_write: number;
      total_tokens: number;
      cost: number;
      model: string;
    }
  | { type: 'phase'; phase: string }
  | {
      type: 'attachment';
      tool_call_id: string | null;
      mime_type: string;
      filename: string | null;
      data_base64: string;
    }
  | {
      type: 'approval_requested';
      id: string;
      tool_name: string;
      summary: string;
      risk_level: string;
      requested_at: string;
      timeout_secs: number;
    }
  | { type: 'approval_resolved'; id: string; decision: string };

/**
 * Synthetic lifecycle frames the stream injects before opening / after
 * closing the WebSocket. The double-underscore prefix is reserved here so
 * they cannot collide with backend events.
 */
type StreamLifecycleEvent = { type: '__stream_started' } | { type: '__stream_closed' };

/**
 * Shape of events the chat consumer receives — raw WebSocket frames plus
 * the two synthetic lifecycle frames so observers can distinguish run
 * boundaries from in-stream content.
 */
export type PublicWebEvent = WebEvent | StreamLifecycleEvent;

/**
 * Derive the WebSocket URL from the configured API base URL.
 *
 * Resolution order mirrors REST (`resolveUrl` in `api/client.ts`):
 * 1. If the user has set a custom `rara_backend_url` in localStorage we
 *    derive WS from that host so REST and WS target the same backend.
 *    Without this, REST follows the override but WS always fell back to
 *    `window.location`, producing "WebSocket connection error" whenever
 *    the override pointed at a remote backend (issue #1622).
 * 2. Otherwise honour an explicit compile-time `BASE_URL`.
 * 3. Otherwise derive from the current page (Vite dev proxy path).
 */
export function buildWsUrl(sessionKey: string): string {
  let base: string;

  const override = typeof window !== 'undefined' ? localStorage.getItem('rara_backend_url') : null;

  if (override) {
    base = getBackendUrl().replace(/^http/, 'ws');
  } else if ((BASE_URL as string).length > 0) {
    base = (BASE_URL as string).replace(/^http/, 'ws');
  } else {
    const loc = window.location;
    const proto = loc.protocol === 'https:' ? 'wss:' : 'ws:';
    base = `${proto}//${loc.host}`;
  }

  // Strip trailing slash so the joined path has exactly one separator.
  base = base.replace(/\/$/, '');

  const user = getAuthUser();
  if (!user) {
    // No authenticated principal — caller must log in before opening a WS.
    // `redirectToLogin` will clear any stale token and navigate to /login.
    redirectToLogin();
    throw new Error('not authenticated');
  }

  // Identity is NOT sent as a query parameter — the backend derives the
  // user id from the authenticated owner token (state.owner_user_id).
  // Previously sending `user_id=...` here clashed with the server-trusted
  // identity and caused `identity resolution failed` errors.
  const token = getAccessToken();
  const params = new URLSearchParams({ session_key: sessionKey });
  if (token) params.set('token', token);
  return `${base}/api/v1/kernel/chat/ws?${params.toString()}`;
}
