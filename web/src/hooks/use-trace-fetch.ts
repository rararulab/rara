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
 * Lazy fetch hooks for the per-turn execution + cascade traces.
 *
 * Both hooks are gated on `enabled` so the network call only fires once
 * the corresponding modal opens — there is no point pre-fetching for
 * every rendered turn (the topology page has dozens) when the user only
 * inspects a few. Caching is per `(sessionKey, seq, kind)` so re-opening
 * the same modal within the same session reuses the result.
 */

import { useQuery } from '@tanstack/react-query';

import type { CascadeTrace, ExecutionTrace } from '@/api/kernel-types';
import { fetchCascadeTrace, fetchExecutionTrace } from '@/api/sessions';

/**
 * Fetch the execution trace for a `(sessionKey, seq)` pair.
 *
 * `enabled` defaults to true; pass `false` when the consuming modal is
 * closed so we do not run the query while the affordance is unused.
 */
export function useExecutionTrace(sessionKey: string, seq: number | null, enabled: boolean) {
  return useQuery<ExecutionTrace>({
    queryKey: ['trace', 'execution', sessionKey, seq] as const,
    queryFn: ({ signal }) => {
      // `enabled` already guards on `seq !== null`, but TS needs the narrowing.
      if (seq === null) return Promise.reject(new Error('seq is null'));
      return fetchExecutionTrace(sessionKey, seq, { signal });
    },
    enabled: enabled && seq !== null,
    // The trace for a completed turn is immutable, so cache aggressively
    // — re-opening the modal within the session should not re-hit the API.
    staleTime: Infinity,
    gcTime: 5 * 60_000,
    retry: false,
  });
}

/** Cascade-trace twin of {@link useExecutionTrace}. */
export function useCascadeTrace(sessionKey: string, seq: number | null, enabled: boolean) {
  return useQuery<CascadeTrace>({
    queryKey: ['trace', 'cascade', sessionKey, seq] as const,
    queryFn: ({ signal }) => {
      if (seq === null) return Promise.reject(new Error('seq is null'));
      return fetchCascadeTrace(sessionKey, seq, { signal });
    },
    enabled: enabled && seq !== null,
    staleTime: Infinity,
    gcTime: 5 * 60_000,
    retry: false,
  });
}
