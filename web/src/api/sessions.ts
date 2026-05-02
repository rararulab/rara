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

import { api } from './client';
import type { CascadeTrace, ExecutionTrace } from './kernel-types';
import type { ChatMessageData, ChatSession, SessionStatus } from './types';

/**
 * One hit returned by `GET /api/v1/chat/sessions/search`.
 *
 * The `snippet` is produced server-side: the backend HTML-escapes the
 * matched text and wraps the query term in `<mark>…</mark>`. Callers
 * that render it with `dangerouslySetInnerHTML` are trusting the backend
 * to produce this exact shape — do not pass user input through here.
 */
export interface SessionSearchHit {
  session_key: string;
  session_title: string;
  snippet: string;
  role: 'user' | 'assistant' | 'other';
  timestamp_ms: number;
  seq: number;
}

interface SessionSearchResponse {
  hits: SessionSearchHit[];
}

/**
 * Search the user's chat sessions for a free-text query.
 *
 * An empty query returns an empty array without hitting the backend.
 * Callers are responsible for debouncing input before calling this.
 */
export async function searchSessions(
  q: string,
  limit = 20,
  options?: { signal?: AbortSignal },
): Promise<SessionSearchHit[]> {
  const trimmed = q.trim();
  if (!trimmed) return [];
  const params = new URLSearchParams({ q: trimmed, limit: String(limit) });
  const res = await api.get<SessionSearchResponse>(
    `/api/v1/chat/sessions/search?${params.toString()}`,
    options?.signal ? { signal: options.signal } : undefined,
  );
  return res.hits;
}

/**
 * Update the per-session archive bit via
 * `PATCH /api/v1/chat/sessions/{key}` (issue #2043).
 *
 * Sends a single-field PATCH body — the backend's double-option
 * deserialiser preserves "leave alone" for every other field. The
 * caller is expected to refresh the session list after the call
 * resolves; this helper does not touch react-query cache state on its
 * own to keep it usable from non-React paths.
 */
export async function updateSessionStatus(
  sessionKey: string,
  status: SessionStatus,
  options?: { signal?: AbortSignal },
): Promise<ChatSession> {
  const path = `/api/v1/chat/sessions/${encodeURIComponent(sessionKey)}`;
  return api.patch<ChatSession>(
    path,
    { status },
    options?.signal ? { signal: options.signal } : undefined,
  );
}

/**
 * Fetch persisted chat messages for a session via
 * `GET /api/v1/chat/sessions/{key}/messages`.
 *
 * The backend reduces the session's tape into a flat `ChatMessage[]` with
 * monotonic `seq`. Default `limit` of 200 mirrors the backend default —
 * see `crates/extensions/backend-admin/src/chat/router.rs`.
 */
export async function listMessages(
  sessionKey: string,
  limit = 200,
  options?: { signal?: AbortSignal },
): Promise<ChatMessageData[]> {
  const params = new URLSearchParams({ limit: String(limit) });
  const path = `/api/v1/chat/sessions/${encodeURIComponent(sessionKey)}/messages?${params.toString()}`;
  return api.get<ChatMessageData[]>(path, options?.signal ? { signal: options.signal } : undefined);
}

/**
 * Fetch the chat-message slice between two anchors on a session.
 *
 * Wraps `GET /api/v1/chat/sessions/{key}/messages?from_anchor=&to_anchor=`.
 * The backend resolves anchor ids against the session row's persisted
 * `anchors[]` to a half-open `[from.byte_offset, to.byte_offset)` byte
 * range and reads only that segment via a seek-based store primitive
 * (issue #2040). Either bound is independently optional:
 *
 * - `to_anchor` omitted → reads to EOF (most-recent anchor case).
 * - `from_anchor` omitted → reads from the start of the tape.
 *
 * Both omitted is supported by the route but pointless via this helper —
 * use `listMessages` instead, which preserves the legacy `?limit` path.
 *
 * Returns the same `ChatMessageData[]` envelope as `listMessages` so the
 * caller can swap the message list in place without per-mode shape
 * branching.
 */
export async function fetchSessionMessagesBetweenAnchors(
  sessionKey: string,
  fromAnchorId: number | null,
  toAnchorId: number | null,
  options?: { signal?: AbortSignal },
): Promise<ChatMessageData[]> {
  const params = new URLSearchParams();
  if (fromAnchorId !== null) params.set('from_anchor', String(fromAnchorId));
  if (toAnchorId !== null) params.set('to_anchor', String(toAnchorId));
  const path = `/api/v1/chat/sessions/${encodeURIComponent(sessionKey)}/messages?${params.toString()}`;
  return api.get<ChatMessageData[]>(path, options?.signal ? { signal: options.signal } : undefined);
}

/**
 * Fetch the per-turn execution trace for a single assistant turn.
 *
 * Wraps `GET /api/v1/chat/sessions/{key}/execution-trace?seq={seq}`. The
 * backend handler (`get_execution_trace` in
 * `crates/extensions/backend-admin/src/chat/router.rs`) returns the
 * iteration count, model, token usage, plan steps, and per-tool summary
 * for the turn whose final assistant tape entry has the given seq.
 */
export async function fetchExecutionTrace(
  sessionKey: string,
  seq: number,
  options?: { signal?: AbortSignal },
): Promise<ExecutionTrace> {
  const params = new URLSearchParams({ seq: String(seq) });
  const path = `/api/v1/chat/sessions/${encodeURIComponent(sessionKey)}/execution-trace?${params.toString()}`;
  return api.get<ExecutionTrace>(path, options?.signal ? { signal: options.signal } : undefined);
}

/**
 * Fetch the cascade (think → act → observe) trace for a single assistant turn.
 *
 * Wraps `GET /api/v1/chat/sessions/{key}/trace?seq={seq}`. The backend
 * handler (`get_cascade_trace`) replays the tape between the spawning user
 * input and the closing `done` frame and returns the structured tick /
 * entry breakdown rendered by `<CascadeModal>`.
 */
export async function fetchCascadeTrace(
  sessionKey: string,
  seq: number,
  options?: { signal?: AbortSignal },
): Promise<CascadeTrace> {
  const params = new URLSearchParams({ seq: String(seq) });
  const path = `/api/v1/chat/sessions/${encodeURIComponent(sessionKey)}/trace?${params.toString()}`;
  return api.get<CascadeTrace>(path, options?.signal ? { signal: options.signal } : undefined);
}
