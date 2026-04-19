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
 * Session-delete fallback selection.
 *
 * When the user deletes a session from the sidebar, the UI needs to
 * pick the session the viewport should land on next. Historically the
 * parent component spawned a brand-new empty session whenever the
 * active row was deleted, which — if any listener echoed the event
 * back into "create a new session" — would loop forever and trap the
 * user in freshly-generated empty chats.
 *
 * The fix is to prefer a neighbour in the still-loaded list. Only when
 * the list is completely empty may the caller fall back to creating
 * one. Extracted into a pure function so the regression is covered by
 * a unit test instead of relying on integration behaviour.
 */

/** Minimum shape needed to pick a fallback — matches `ChatSession["key"]`. */
export interface HasKey {
  key: string;
}

/**
 * Choose the session the sidebar should switch into after `deletedKey`
 * is removed, given the pre-deletion `sessions` ordering. Returns
 * `null` when there are no other sessions left.
 *
 * Preference order: next neighbour in the list, then previous
 * neighbour. Mirrors the order the Kimi-style sidebar renders, so the
 * user always stays near the row they just deleted.
 */
export function pickSessionFallback<T extends HasKey>(
  sessions: readonly T[],
  deletedKey: string,
): T | null {
  const idx = sessions.findIndex((s) => s.key === deletedKey);
  if (idx < 0) return null;
  return sessions[idx + 1] ?? sessions[idx - 1] ?? null;
}

/** Action the parent should take after a session is deleted. */
export type PostDeleteAction<T> =
  | { kind: 'noop' }
  | { kind: 'switch'; session: T }
  | { kind: 'create-new' };

/**
 * Decide what the chat page should do when the sidebar reports a
 * deletion. Pure so the infinite-loop regression (create-new firing
 * while other sessions still exist) is covered by a unit test.
 *
 * - Unrelated deletion → noop (stay on the current session).
 * - Active session deleted with a fallback → switch.
 * - Active session deleted with no fallback → create-new.
 */
export function decidePostDeleteAction<T extends HasKey>(params: {
  activeSessionKey: string | undefined;
  deletedKey: string;
  fallback: T | null;
}): PostDeleteAction<T> {
  if (params.activeSessionKey !== params.deletedKey) {
    return { kind: 'noop' };
  }
  if (params.fallback) {
    return { kind: 'switch', session: params.fallback };
  }
  return { kind: 'create-new' };
}
