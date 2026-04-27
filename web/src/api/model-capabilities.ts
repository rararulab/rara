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
 * Model capability lookup — backed by `GET /api/v1/chat/models`.
 *
 * Used by `RaraAgent.prompt()` to pre-flight image attachments: when the
 * selected model has `supports_vision === false` we refuse the send so the
 * kernel never silently drops the image block (which would otherwise be
 * replaced by a `[image: current model does not support vision]`
 * placeholder at request build time).
 *
 * The catalog rarely changes during a session, so we cache the result of
 * the first fetch indefinitely. Callers that need a fresh view (e.g. after
 * favoriting/unfavoriting) call `refreshModelCapabilities()`.
 */

import { api } from './client';

interface ChatModelDto {
  id: string;
  name: string;
  context_length: number;
  is_favorite: boolean;
  supports_vision: boolean;
}

/** Lookup from model id to whether the model accepts image input. */
export type ModelCapabilityMap = Map<string, boolean>;

let inflight: Promise<ModelCapabilityMap> | null = null;

async function fetchCapabilityMap(): Promise<ModelCapabilityMap> {
  const models = await api.get<ChatModelDto[]>('/api/v1/chat/models');
  const map: ModelCapabilityMap = new Map();
  for (const m of models) {
    map.set(m.id, m.supports_vision);
  }
  return map;
}

/**
 * Return the cached capability map, fetching once on first call.
 *
 * Failures (network, auth) reject the promise and clear the cached
 * promise so the next caller can retry. This is intentional: the gate
 * downstream is fail-open (unknown model id → allow send) so a single
 * fetch failure must not permanently block image sends.
 */
export function getModelCapabilities(): Promise<ModelCapabilityMap> {
  if (!inflight) {
    inflight = fetchCapabilityMap().catch((err) => {
      inflight = null;
      throw err;
    });
  }
  return inflight;
}

/** Force a re-fetch on the next `getModelCapabilities()` call. */
export function refreshModelCapabilities(): void {
  inflight = null;
}
