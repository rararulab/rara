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
import type { ChatMessageData } from './types';

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
