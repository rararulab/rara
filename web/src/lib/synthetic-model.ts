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

import type { Model } from "@mariozechner/pi-ai";

/**
 * Build a pi-ai [`Model`] shape from rara's own provider + model pair.
 *
 * pi-chat-panel consumes `agent.state.model` only for UI display (the
 * model-name pill in the composer) and local bookkeeping — streaming
 * bypasses pi-ai entirely and rides on rara's WebSocket. The
 * synthesized fields (`api`, `baseUrl`, `cost`, `contextWindow`) need
 * only be structurally valid; their values never hit the wire.
 */
export function syntheticModel(
  providerId: string,
  modelId: string,
  options?: { baseUrl?: string; contextWindow?: number; name?: string },
): Model<any> {
  return {
    id:            modelId,
    name:          options?.name ?? `${providerId} / ${modelId}`,
    api:           "openai-completions",
    provider:      providerId,
    baseUrl:       options?.baseUrl ?? "",
    reasoning:     false,
    input:         ["text", "image"],
    cost:          { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    contextWindow: options?.contextWindow ?? 128_000,
    maxTokens:     4096,
  } as Model<any>;
}

/** Sentinel model-id/provider assigned by pi-agent-core before any pick. */
export const UNKNOWN_MODEL_SENTINEL = "unknown";

/**
 * True when `agent.state.model` is pi-agent-core's placeholder default,
 * i.e. the user has not selected a model and no session state has been
 * restored. Callers use this to avoid persisting a bogus override.
 */
export function isUnknownModel(model: {
  id?: string;
  provider?: string;
} | null | undefined): boolean {
  if (!model) return true;
  return (
    model.id === UNKNOWN_MODEL_SENTINEL
    || model.provider === UNKNOWN_MODEL_SENTINEL
  );
}
