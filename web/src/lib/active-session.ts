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
 * Shared persistence key + helpers for the "currently active chat session"
 * used by both `PiChat` and `PiChatV2`. PR7 deletes the legacy page; this
 * module remains as the single source of truth for the storage key.
 */

export const ACTIVE_SESSION_KEY = 'rara.activeSessionKey';

/** Read the stored session key from localStorage, returning null on any
 *  failure (storage disabled, quota exceeded, etc). */
export function readStoredSessionKey(): string | null {
  try {
    return localStorage.getItem(ACTIVE_SESSION_KEY);
  } catch {
    return null;
  }
}

/** Persist the active session key. Pass `null` to clear it. */
export function writeStoredSessionKey(key: string | null): void {
  try {
    if (key) localStorage.setItem(ACTIVE_SESSION_KEY, key);
    else localStorage.removeItem(ACTIVE_SESSION_KEY);
  } catch {
    /* ignore */
  }
}
