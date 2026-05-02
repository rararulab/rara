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

import { createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react';

/**
 * Coarse live-indicator state surfaced by a page to the slim top bar.
 *
 * `null` = no indicator (route doesn't show one, or the page isn't
 * actively subscribed). Pages that own a long-lived subscription (today
 * only `Chat.tsx`) publish their state here so the layout can render a
 * single "live" pill without re-opening the WebSocket — re-subscribing
 * from the layout would double the WS load and could race with the
 * page's own reconnect logic.
 */
export type PageLiveStatus = 'idle' | 'connecting' | 'live' | 'reconnecting' | 'closed';

interface PageStatusContextValue {
  status: PageLiveStatus | null;
  setStatus: (next: PageLiveStatus | null) => void;
}

const PageStatusContext = createContext<PageStatusContextValue>({
  status: null,
  setStatus: () => {},
});

/** Provider mounted near the layout root so any descendant page can publish. */
export function PageStatusProvider({ children }: { children: React.ReactNode }) {
  const [status, setStatus] = useState<PageLiveStatus | null>(null);
  const value = useMemo<PageStatusContextValue>(() => ({ status, setStatus }), [status]);
  return <PageStatusContext.Provider value={value}>{children}</PageStatusContext.Provider>;
}

/** Read-only access for the layout / top bar. */
export function usePageStatus(): PageLiveStatus | null {
  return useContext(PageStatusContext).status;
}

/**
 * Page-side hook: publish a live status while mounted, clear on unmount.
 *
 * Pass `null` when the page knows it has no session to report on so the
 * top bar collapses the indicator instead of showing a stale value.
 */
export function usePublishPageStatus(status: PageLiveStatus | null): void {
  const { setStatus } = useContext(PageStatusContext);
  // Stable setter via useCallback so the effect below only fires when the
  // status itself changes, not on every parent render.
  const publish = useCallback(
    (next: PageLiveStatus | null) => {
      setStatus(next);
    },
    [setStatus],
  );
  useEffect(() => {
    publish(status);
    return () => {
      publish(null);
    };
  }, [publish, status]);
}
