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

import { api } from "@/api/client";
import type {
  ChatMessageData,
  ChatModel,
  ChatSession,
} from "@/api/types";

// ---------------------------------------------------------------------------
// API helpers
// ---------------------------------------------------------------------------

export function fetchSessions() {
  return api.get<ChatSession[]>("/api/v1/chat/sessions?limit=100&offset=0");
}

export function fetchMessages(key: string) {
  return api.get<ChatMessageData[]>(
    `/api/v1/chat/sessions/${encodeURIComponent(key)}/messages?limit=200`,
  );
}

export function createSession(body: {
  key: string;
  title?: string;
  model?: string;
  system_prompt?: string;
}) {
  return api.post<ChatSession>("/api/v1/chat/sessions", body);
}

export function deleteSession(key: string) {
  return api.del<void>(
    `/api/v1/chat/sessions/${encodeURIComponent(key)}`,
  );
}

export function clearMessages(key: string) {
  return api.del<void>(
    `/api/v1/chat/sessions/${encodeURIComponent(key)}/messages`,
  );
}

export function fetchModels() {
  return api.get<ChatModel[]>("/api/v1/chat/models");
}

export function setFavoriteModels(modelIds: string[]) {
  return api.put<string[]>("/api/v1/chat/models/favorites", {
    model_ids: modelIds,
  });
}

export function updateSession(
  key: string,
  body: { title?: string; model?: string; system_prompt?: string },
) {
  return api.patch<ChatSession>(
    `/api/v1/chat/sessions/${encodeURIComponent(key)}`,
    body,
  );
}
