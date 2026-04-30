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

import { Network } from 'lucide-react';
import { type FormEvent, useState } from 'react';
import { useNavigate, useParams } from 'react-router';

import { TimelineView } from '@/components/topology/TimelineView';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import {
  useTopologySubscription,
  type TopologyStatus,
} from '@/hooks/use-topology-subscription';

/**
 * Multi-agent observability page — main timeline view (task #5 of #1999).
 *
 * URL: `/topology` (no session) or `/topology/:rootSessionKey`.
 *
 * The page wires the cross-session topology WebSocket
 * (`/api/v1/kernel/chat/topology/{root}`) to a `TimelineView` that
 * renders the root session's stream of consciousness as a vertical
 * sequence of turn cards. Descendant-session events flow through the
 * same socket but are not rendered here; tasks #6 (worker inbox) and
 * #7 (fork topology) consume them in subsequent steps.
 */
export default function Topology() {
  const { rootSessionKey } = useParams<{ rootSessionKey?: string }>();
  const navigate = useNavigate();
  const [draft, setDraft] = useState(rootSessionKey ?? '');

  const subscription = useTopologySubscription(rootSessionKey ?? null);

  const handleConnect = (e: FormEvent) => {
    e.preventDefault();
    const trimmed = draft.trim();
    if (!trimmed) return;
    void navigate(`/topology/${encodeURIComponent(trimmed)}`);
  };

  return (
    <div className="flex h-full flex-col gap-3">
      <div className="flex items-center gap-3">
        <div className="flex items-center gap-2">
          <Network className="h-4 w-4 text-muted-foreground" />
          <h1 className="text-sm font-medium">Topology</h1>
        </div>

        <form onSubmit={handleConnect} className="flex flex-1 items-center gap-2">
          <Input
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            placeholder="root session key — e.g. mita::01HX…"
            className="h-8 max-w-md font-mono text-xs"
          />
          <Button type="submit" size="sm" variant="secondary" className="h-8">
            Connect
          </Button>
        </form>

        <StatusPill status={subscription.status} />
      </div>

      {rootSessionKey ? (
        <div className="flex flex-1 min-h-0 gap-3">
          <div className="flex flex-1 min-w-0 flex-col overflow-auto">
            <TimelineView
              rootSessionKey={rootSessionKey}
              events={subscription.events}
            />
          </div>

          <aside className="hidden w-64 shrink-0 flex-col gap-2 border-l pl-3 md:flex">
            <div className="text-xs font-medium text-muted-foreground">Sessions</div>
            <div className="space-y-1">
              {[...subscription.sessions.entries()].map(([key, parent]) => (
                <div
                  key={key}
                  className="flex flex-col rounded border bg-muted/20 px-2 py-1.5 text-[11px]"
                >
                  <span className="truncate font-mono text-foreground">{key}</span>
                  {parent && (
                    <span className="truncate font-mono text-muted-foreground">
                      ↳ from {parent}
                    </span>
                  )}
                </div>
              ))}
            </div>
            <div className="mt-auto text-[10px] text-muted-foreground">
              Worker inbox arrives in task #6 — see issue #1999.
            </div>
          </aside>
        </div>
      ) : (
        <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
          Enter a root session key to start observing.
        </div>
      )}
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
