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

import { useCallback } from 'react';

import { decidePostDeleteAction, type HasKey } from '@/lib/session-fallback';

/**
 * Wires the pure `decidePostDeleteAction` decision into the chat page's
 * switch/create-new side effects. Kept as a hook (rather than inlined
 * in `PiChat`) so the switch/create-new dispatch is covered by a unit
 * test — without the hook the wiring was only exercised by the pure
 * helper plus a 45K-line page component.
 *
 * Returns a stable callback with the same signature the sidebar's
 * `onDeleteSession` prop expects.
 */
export function useSessionDelete<T extends HasKey>(deps: {
  activeSessionKey: string | undefined;
  switchSession: (session: T) => void | Promise<void>;
  newSession: () => void | Promise<void>;
}): (deletedKey: string, fallback: T | null) => void {
  const { activeSessionKey, switchSession, newSession } = deps;
  return useCallback(
    (deletedKey, fallback) => {
      const action = decidePostDeleteAction({
        activeSessionKey,
        deletedKey,
        fallback,
      });
      switch (action.kind) {
        case 'noop':
          return;
        case 'switch':
          void switchSession(action.session);
          return;
        case 'create-new':
          void newSession();
          return;
      }
    },
    [activeSessionKey, switchSession, newSession],
  );
}
