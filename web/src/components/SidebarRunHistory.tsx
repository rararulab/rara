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

import { useState } from 'react';

import { AgentTranscriptDialog } from './agent-live/AgentTranscriptDialog';
import type { LiveRun } from './agent-live/live-run-store';
import { TaskRunHistory } from './agent-live/TaskRunHistory';
import { useLiveRun } from './agent-live/use-live-run';

interface Props {
  /** Active session's key; when undefined the section is hidden. */
  activeSessionKey: string | undefined;
}

/**
 * Sidebar-scoped wrapper around {@link TaskRunHistory}.
 *
 * Subscribes to {@link useLiveRun} for the currently active session and
 * renders the completed/failed/cancelled runs produced by the agent
 * live store. Clicking a row's transcript button opens the shared
 * {@link AgentTranscriptDialog}.
 *
 * Rendered inside {@link ChatSidebar} so users can inspect prior runs
 * after the inline live card retired its in-flow history (#1620).
 * Hidden entirely when there is no history — keeps the sidebar quiet
 * for fresh sessions.
 */
export function SidebarRunHistory({ activeSessionKey }: Props) {
  const slice = useLiveRun(activeSessionKey);
  const [openRun, setOpenRun] = useState<LiveRun | null>(null);

  if (slice.history.length === 0) return null;

  return (
    <div className="shrink-0 border-t border-border/30 px-2 py-2">
      <div className="mb-1 px-1 text-[11px] font-medium uppercase tracking-wider text-muted-foreground/80">
        执行历史 ({slice.history.length})
      </div>
      <div className="max-h-64 overflow-y-auto">
        <TaskRunHistory runs={slice.history} onOpenTranscript={setOpenRun} />
      </div>
      <AgentTranscriptDialog
        run={openRun}
        open={openRun !== null}
        onClose={() => setOpenRun(null)}
      />
    </div>
  );
}
