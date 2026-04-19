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

import { useSyncExternalStore } from 'react';

import { EMPTY_SLICE, liveRunStore, type SessionSlice } from './live-run-store';

/**
 * Subscribe to the {@link liveRunStore} slice for a given session key.
 *
 * Returns the current snapshot and re-renders on every publish. When
 * `sessionKey` is undefined, returns the shared frozen `EMPTY_SLICE` —
 * `useSyncExternalStore` bails out of re-rendering only when
 * `getSnapshot` returns a referentially stable value, so we MUST NOT
 * construct a fresh `{ active: null, history: [] }` on every call
 * (that produced React error #185 at the welcome screen).
 */
export function useLiveRun(sessionKey: string | undefined): SessionSlice {
  const subscribe = (listener: () => void): (() => void) => {
    if (!sessionKey) return () => {};
    return liveRunStore.subscribe(sessionKey, listener);
  };
  const getSnapshot = (): SessionSlice =>
    sessionKey ? liveRunStore.snapshot(sessionKey) : EMPTY_SLICE;
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}
