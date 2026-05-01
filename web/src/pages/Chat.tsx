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
import { ArrowLeft, Network, PanelLeft, PanelLeftClose } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import { useNavigate, useParams } from 'react-router';

import { api } from '@/api/client';
import { fetchSessionMessagesBetweenAnchors } from '@/api/sessions';
import type { ChatMessageData, ChatSession, SessionAnchor } from '@/api/types';
import { SessionPicker } from '@/components/topology/SessionPicker';
import { TapeLineageView } from '@/components/topology/TapeLineageView';
import { TimelineChapterStrip } from '@/components/topology/TimelineChapterStrip';
import { TimelineView } from '@/components/topology/TimelineView';
import { WorkerInbox } from '@/components/topology/WorkerInbox';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { useTopologySubscription, type TopologyStatus } from '@/hooks/use-topology-subscription';

/**
 * Multi-agent observability page — craft-style 3-pane shell.
 *
 * Layout:
 *
 * ```
 * ┌──────────────┬────────────────────┬──────────────┐
 * │ SessionPicker│  TimelineView      │ WorkerInbox  │
 * │              │  (selected session)│ + Lineage    │
 * └──────────────┴────────────────────┴──────────────┘
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

  // Currently-selected anchor segment (issue #2040). `null` means "no
  // chapter selected" → `TimelineView` falls back to its own
  // `useSessionHistory` fetch (the legacy "last 200" path).
  const [segmentMessages, setSegmentMessages] = useState<ChatMessageData[] | null>(null);

  // Reset the chapter selection whenever the rendered session changes —
  // a chapter is meaningful only against the tape it was clicked on, and
  // showing stale messages after a session swap would be a worse bug
  // than the extra round-trip on the new session.
  const renderedSessionKey = viewChild ?? rootSessionKey ?? null;
  useEffect(() => {
    setSegmentMessages(null);
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
      <header className="flex items-center gap-3 border-b border-border px-3 py-2">
        <Button
          size="icon"
          variant="ghost"
          className="h-7 w-7"
          aria-label={sidebarCollapsed ? 'Show sidebar' : 'Hide sidebar'}
          title={sidebarCollapsed ? 'Show sidebar' : 'Hide sidebar'}
          onClick={() => setSidebarCollapsed((v) => !v)}
        >
          {sidebarCollapsed ? (
            <PanelLeft className="h-4 w-4" />
          ) : (
            <PanelLeftClose className="h-4 w-4" />
          )}
        </Button>
        <Network className="h-4 w-4 text-muted-foreground" />
        <h1 className="text-sm font-medium">Chat</h1>
        <div className="ml-auto">
          {rootSessionKey ? (
            <StatusPill status={subscription.status} />
          ) : (
            <Badge variant="outline" className="text-[10px]">
              no session
            </Badge>
          )}
        </div>
      </header>

      <div className="flex flex-1 min-h-0">
        {!sidebarCollapsed && (
          <aside className="hidden w-[280px] shrink-0 border-r border-border md:block">
            <SessionPicker
              activeSessionKey={rootSessionKey ?? null}
              onSelect={selectSession}
              onAutoSelect={selectSession}
            />
          </aside>
        )}

        <main className="flex flex-1 min-w-0 min-h-0 flex-col p-3">
          {rootSessionKey ? (
            <div className="flex flex-1 min-h-0 flex-col gap-2">
              {viewChild && (
                <div className="flex items-center gap-2">
                  <Button
                    size="sm"
                    variant="ghost"
                    className="h-7 px-2 text-xs"
                    onClick={() => setViewChild(null)}
                  >
                    <ArrowLeft className="mr-1 h-3 w-3" />
                    back to root
                  </Button>
                  <span className="truncate font-mono text-[11px] text-muted-foreground">
                    viewing {viewChild}
                  </span>
                </div>
              )}
              <TimelineChapterStrip anchors={anchors} onSelectAnchor={handleSelectAnchor} />
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
        </main>

        {rootSessionKey && (
          <aside className="hidden w-[320px] shrink-0 flex-col gap-3 overflow-y-auto border-l border-border p-3 lg:flex">
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
              <div className="mb-1.5 text-xs font-medium text-muted-foreground">Tape lineage</div>
              <TapeLineageView
                events={subscription.events}
                activeSessionKey={viewChild ?? rootSessionKey}
              />
            </div>
          </aside>
        )}
      </div>
    </div>
  );
}

function StatusPill({ status }: { status: TopologyStatus }) {
  switch (status.kind) {
    case 'idle':
      return (
        <Badge variant="outline" className="text-[10px]">
          idle
        </Badge>
      );
    case 'connecting':
      return (
        <Badge variant="outline" className="text-[10px]">
          connecting…
        </Badge>
      );
    case 'open':
      return (
        <Badge
          variant="outline"
          className="border-emerald-500/40 text-[10px] text-emerald-600 dark:text-emerald-400"
        >
          live
        </Badge>
      );
    case 'reconnecting':
      return (
        <Badge variant="outline" className="text-[10px]">
          reconnect #{status.attempt} ({Math.round(status.delayMs / 1000)}s)
        </Badge>
      );
    case 'closed':
      return (
        <Badge variant="destructive" className="text-[10px]">
          closed: {status.reason.replace(/_/g, ' ')}
        </Badge>
      );
  }
}
