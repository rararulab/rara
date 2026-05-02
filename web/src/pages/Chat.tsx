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

import { useQuery } from '@tanstack/react-query';
import { ArrowLeft, PanelLeft, PanelLeftClose } from 'lucide-react';
import { AnimatePresence, motion } from 'motion/react';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { useNavigate, useParams } from 'react-router';

import { api } from '@/api/client';
import { fetchSessionMessagesBetweenAnchors } from '@/api/sessions';
import type { ChatMessageData, ChatSession, SessionAnchor } from '@/api/types';
import { usePublishPageStatus, type PageLiveStatus } from '@/components/shell/PageStatusContext';
import { SessionAnchorMinimap } from '@/components/topology/SessionAnchorMinimap';
import { SessionPicker } from '@/components/topology/SessionPicker';
import { TimelineView } from '@/components/topology/TimelineView';
import { WorkerInbox } from '@/components/topology/WorkerInbox';
import { Button } from '@/components/ui/button';
import { useTopologySubscription, type TopologyStatus } from '@/hooks/use-topology-subscription';

/**
 * Multi-agent observability page — craft-style 3-pane shell.
 *
 * Layout:
 *
 * ```
 * ┌──────────────┬────────────────────┬──────────────────┐
 * │ SessionPicker│  TimelineView      │ WorkerInbox      │
 * │              │  (selected session)│ + AnchorMinimap  │
 * └──────────────┴────────────────────┴──────────────────┘
 * ```
 *
 * URL: `/chat` (no session) or `/chat/:rootSessionKey`. The
 * shell auto-selects the most recently updated session on first load
 * so users never have to paste a session UUID — the UX complaint
 * `#1999` task #9 fixes.
 */
/**
 * localStorage key for the per-user collapsed-sidebar preference.
 *
 * The literal `rara.topology.*` namespace is preserved on purpose: the
 * page was renamed from "Topology" to "Chat" in `#2041`, but the storage
 * key is a user-data contract — flipping it would silently reset every
 * existing user's sidebar state on first load after upgrade.
 */
const SIDEBAR_COLLAPSED_STORAGE_KEY = 'rara.topology.sidebarCollapsed';

/**
 * Shared spring transition tuned per the polish checklist (#2042) — the
 * `bounce: 0` constraint is load-bearing: anything above 0 reads as a
 * cartoon overshoot on a UI surface this dense.
 */
const SPRING = { type: 'spring', duration: 0.3, bounce: 0 } as const;

/**
 * Read the persisted collapsed-sidebar preference, swallowing access
 * errors (private browsing, disabled storage). The default is `false`
 * so first-time visitors still see the picker.
 */
function readSidebarCollapsed(): boolean {
  try {
    return window.localStorage.getItem(SIDEBAR_COLLAPSED_STORAGE_KEY) === 'true';
  } catch {
    return false;
  }
}

export default function Chat() {
  const { rootSessionKey } = useParams<{ rootSessionKey?: string }>();
  const navigate = useNavigate();
  // Which session the main timeline shows. `null` = root view; a child
  // session key focuses on that worker. Reset whenever the root changes
  // so a new connection always lands on the root view.
  const [viewChild, setViewChild] = useState<string | null>(null);
  // Lazy initializer reads localStorage exactly once on mount; the
  // useEffect below mirrors the React state back to storage on change.
  const [sidebarCollapsed, setSidebarCollapsed] = useState<boolean>(readSidebarCollapsed);

  useEffect(() => {
    try {
      window.localStorage.setItem(
        SIDEBAR_COLLAPSED_STORAGE_KEY,
        sidebarCollapsed ? 'true' : 'false',
      );
    } catch {
      // Storage may be unavailable (private browsing, quota); the toggle
      // still works in-memory for the rest of the session.
    }
  }, [sidebarCollapsed]);

  const subscription = useTopologySubscription(rootSessionKey ?? null);

  // Publish the WS status up to the slim top bar so the layout can show
  // the live pill without re-subscribing. `null` while no session is
  // selected — the indicator is not meaningful before a connection is
  // even attempted.
  const publishedStatus = useMemo<PageLiveStatus | null>(() => {
    if (!rootSessionKey) return null;
    return mapTopologyStatusToPageStatus(subscription.status);
  }, [rootSessionKey, subscription.status]);
  usePublishPageStatus(publishedStatus);

  // Currently-selected anchor segment (issue #2040). `null` means "no
  // chapter selected" → `TimelineView` falls back to its own
  // `useSessionHistory` fetch (the legacy "last 200" path).
  const [segmentMessages, setSegmentMessages] = useState<ChatMessageData[] | null>(null);
  // `anchor_id` of the segment whose messages are currently rendered, so
  // the right-rail minimap can highlight "you are here". `null` while the
  // user has not picked a chapter — the minimap then falls back to the
  // most-recent anchor as the implicit current position.
  const [currentAnchorId, setCurrentAnchorId] = useState<number | null>(null);

  // Reset the chapter selection whenever the rendered session changes —
  // a chapter is meaningful only against the tape it was clicked on, and
  // showing stale messages after a session swap would be a worse bug
  // than the extra round-trip on the new session.
  const renderedSessionKey = viewChild ?? rootSessionKey ?? null;
  useEffect(() => {
    setSegmentMessages(null);
    setCurrentAnchorId(null);
  }, [renderedSessionKey]);

  // Fetch the session row to get `anchors[]` for the chapter strip.
  // This is a separate query from the picker's list so the strip can
  // mount independently and the cache stays scoped per-session.
  const sessionQuery = useQuery<ChatSession | null>({
    queryKey: ['topology', 'session', renderedSessionKey] as const,
    queryFn: ({ signal }) => {
      if (!renderedSessionKey) return Promise.resolve(null);
      return api.get<ChatSession>(
        `/api/v1/chat/sessions/${encodeURIComponent(renderedSessionKey)}`,
        signal ? { signal } : undefined,
      );
    },
    enabled: renderedSessionKey !== null,
    staleTime: 30_000,
  });
  const anchors: SessionAnchor[] = sessionQuery.data?.anchors ?? [];

  const handleSelectAnchor = useCallback(
    (from: SessionAnchor, to: SessionAnchor | null) => {
      if (!renderedSessionKey) return;
      setCurrentAnchorId(from.anchor_id);
      void fetchSessionMessagesBetweenAnchors(
        renderedSessionKey,
        from.anchor_id,
        to?.anchor_id ?? null,
      ).then((messages) => {
        setSegmentMessages(messages);
      });
    },
    [renderedSessionKey],
  );

  useEffect(() => {
    setViewChild(null);
  }, [rootSessionKey]);

  const selectSession = useCallback(
    (key: string) => {
      void navigate(`/chat/${encodeURIComponent(key)}`);
    },
    [navigate],
  );

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex flex-1 min-h-0">
        {/*
         * Sidebar collapse uses a spring on `width` (interruptible per
         * principle #4). `AnimatePresence initial={false}` keeps the
         * first render quiet — only later toggles animate.
         */}
        <AnimatePresence initial={false}>
          {!sidebarCollapsed && (
            <motion.aside
              key="sidebar"
              initial={{ width: 0, opacity: 0 }}
              animate={{ width: 280, opacity: 1 }}
              exit={{ width: 0, opacity: 0 }}
              transition={SPRING}
              className="hidden shrink-0 overflow-hidden border-r border-border md:block"
            >
              <div className="relative h-full w-[280px]">
                {/*
                 * Per-page sessions-column collapse affordance. Floats
                 * over the picker's own header (no extra row of chrome
                 * stacked on top). The toggle used to live in the
                 * page-level chrome before #2059 moved app chrome into
                 * the global rail.
                 */}
                <Button
                  size="icon"
                  variant="ghost"
                  className="absolute right-1.5 top-1.5 z-10 h-7 w-7 text-muted-foreground transition-transform hover:text-foreground active:scale-[0.96]"
                  aria-label="Hide sessions"
                  title="Hide sessions"
                  onClick={() => setSidebarCollapsed(true)}
                >
                  <PanelLeftClose className="h-4 w-4" />
                </Button>
                <SessionPicker
                  activeSessionKey={rootSessionKey ?? null}
                  onSelect={selectSession}
                  onAutoSelect={selectSession}
                />
              </div>
            </motion.aside>
          )}
        </AnimatePresence>

        {/*
         * Stagger entrance per principle #5 — main lands ~100ms after
         * the sidebar so the eye reaches the timeline second.
         */}
        <motion.main
          initial={{ opacity: 0, y: 4 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ ...SPRING, delay: 0.1 }}
          className="flex flex-1 min-w-0 min-h-0 flex-col p-3"
        >
          {sidebarCollapsed && (
            <div className="mb-2 hidden md:flex">
              <Button
                size="icon"
                variant="ghost"
                className="h-7 w-7 transition-transform active:scale-[0.96]"
                aria-label="Show sessions"
                title="Show sessions"
                onClick={() => setSidebarCollapsed(false)}
              >
                <PanelLeft className="h-4 w-4" />
              </Button>
            </div>
          )}
          {rootSessionKey ? (
            <div className="flex flex-1 min-h-0 flex-col gap-2">
              {viewChild && (
                <div className="flex items-center gap-2">
                  <Button
                    size="sm"
                    variant="ghost"
                    className="h-7 px-2 text-xs transition-transform active:scale-[0.96]"
                    onClick={() => setViewChild(null)}
                  >
                    <ArrowLeft className="mr-1 h-3 w-3" />
                    back to root
                  </Button>
                  <span className="truncate font-mono text-[11px] tabular-nums text-muted-foreground">
                    viewing {viewChild}
                  </span>
                </div>
              )}
              <TimelineView
                viewSessionKey={viewChild ?? rootSessionKey}
                events={subscription.events}
                // The editor always sends into the root session — sending
                // into a worker child would write to a sandbox tape that
                // the user did not pick. Browsing a child via the worker
                // inbox is observation-only; replies still go to root.
                promptSessionKey={rootSessionKey}
                segmentMessages={segmentMessages}
              />
            </div>
          ) : (
            <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
              Select a session from the left, or create a new one to start observing.
            </div>
          )}
        </motion.main>

        {rootSessionKey && (
          <motion.aside
            // Right rail enters last (~200ms delay) — staggered with
            // sidebar + main per principle #5.
            initial={{ opacity: 0, y: 4 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ ...SPRING, delay: 0.2 }}
            className="hidden w-[320px] shrink-0 flex-col gap-3 overflow-y-auto border-l border-border p-3 lg:flex"
          >
            <div>
              <div className="mb-1.5 text-xs font-medium text-muted-foreground">Workers</div>
              <WorkerInbox
                rootSessionKey={rootSessionKey}
                events={subscription.events}
                activeChildSession={viewChild}
                onSelectChild={setViewChild}
              />
            </div>
            <div>
              <div className="mb-1.5 text-xs font-medium text-muted-foreground">Anchors</div>
              <SessionAnchorMinimap
                anchors={anchors}
                currentAnchorId={currentAnchorId}
                onSelectAnchor={handleSelectAnchor}
              />
            </div>
          </motion.aside>
        )}
      </div>
    </div>
  );
}

/**
 * Map the per-session WS status to the coarse `PageLiveStatus` consumed
 * by the slim top bar. The granular `attempt` / `delayMs` info on
 * `reconnecting` and the `reason` on `closed` are intentionally dropped —
 * the layout-level pill only carries a single-word state (#2059); pages
 * that need the detail can render their own badge inline.
 */
function mapTopologyStatusToPageStatus(status: TopologyStatus): PageLiveStatus {
  switch (status.kind) {
    case 'idle':
      return 'idle';
    case 'connecting':
      return 'connecting';
    case 'open':
      return 'live';
    case 'reconnecting':
      return 'reconnecting';
    case 'closed':
      return 'closed';
  }
}
