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
 * Cached fetch of the available LLM models from `GET /api/v1/chat/models`.
 *
 * Mirrors the model picker surface used by the legacy chat page. The
 * backend caches its provider response for 5 minutes so a long
 * `staleTime` here costs nothing.
 */

import { useQuery } from '@tanstack/react-query';

import { api } from '@/api/client';

/** Wire shape of `GET /api/v1/chat/models` — keep in sync with
 *  `crates/extensions/backend-admin/src/chat/model_catalog.rs::ChatModel`. */
export interface ChatModelInfo {
  id: string;
  name: string;
  context_length: number;
  is_favorite: boolean;
  supports_vision: boolean;
}

const MODELS_QUERY_KEY = ['topology', 'chat-models'] as const;

/** 5 minutes — matches the backend cache TTL. */
const STALE_MS = 5 * 60 * 1000;

export function useChatModels() {
  return useQuery({
    queryKey: MODELS_QUERY_KEY,
    queryFn: () => api.get<ChatModelInfo[]>('/api/v1/chat/models'),
    staleTime: STALE_MS,
  });
}
