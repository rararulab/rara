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
 * Fetch the persisted chat history for a session via
 * `GET /api/v1/chat/sessions/{key}/messages`.
 *
 * Used by `TimelineView` to render the conversation that already exists
 * on disk before any live topology event arrives. History mutates on
 * every turn, so `staleTime: 0` — the WS push keeps the live tail fresh
 * and any remount will refetch.
 */

import { useQuery } from '@tanstack/react-query';

import { listMessages } from '@/api/sessions';
import type { ChatMessageData } from '@/api/types';

/**
 * Subscribe to a session's persisted message history.
 *
 * Pass `null` to disable the query (e.g. before a session is selected).
 * The cache key includes the session key so switching sessions swaps the
 * cached result and triggers a fetch for the new key automatically.
 */
export function useSessionHistory(sessionKey: string | null) {
  return useQuery<ChatMessageData[]>({
    queryKey: ['topology', 'session-history', sessionKey] as const,
    queryFn: ({ signal }) => {
      // `enabled: !!sessionKey` keeps queryFn from running when null, but
      // TS still wants the parameter narrowed.
      if (!sessionKey) return Promise.resolve([]);
      return listMessages(sessionKey, 200, { signal });
    },
    enabled: sessionKey !== null,
    staleTime: 0,
  });
}
