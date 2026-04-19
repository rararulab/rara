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

import { AgentTranscriptDialog } from './AgentTranscriptDialog';
import type { LiveRun } from './live-run-store';
import { SingleAgentLiveCard } from './SingleAgentLiveCard';
import { TaskRunHistory } from './TaskRunHistory';
import { useLiveRun } from './use-live-run';

interface Props {
  sessionKey: string | undefined;
}

/**
 * Top-level sticky card shown above the chat transcript while an agent
 * run is active, plus an `Execution history` section for prior runs.
 * Stop is intentionally wired as `disabled` — the backend cancel
 * endpoint is not implemented yet (tracked in a follow-up issue).
 */
export function AgentLiveCard({ sessionKey }: Props) {
  const slice = useLiveRun(sessionKey);
  const [openRun, setOpenRun] = useState<LiveRun | null>(null);

  const hasAnything = slice.active !== null || slice.history.length > 0;
  if (!hasAnything) return null;

  return (
    <div className="sticky top-0 z-30 flex flex-col gap-2 border-b border-border/40 bg-background/80 px-3 py-2 backdrop-blur">
      {slice.active && (
        <SingleAgentLiveCard
          run={slice.active}
          onOpenTranscript={() => setOpenRun(slice.active)}
          // Stop endpoint not yet wired — see follow-up issue referenced
          // in the PR body. Passing no handler disables the button with
          // a clarifying tooltip inside SingleAgentLiveCard.
          {...({} as { onStop?: () => void })}
        />
      )}
      <TaskRunHistory runs={slice.history} onOpenTranscript={setOpenRun} />
      <AgentTranscriptDialog
        run={openRun}
        open={openRun !== null}
        onClose={() => setOpenRun(null)}
      />
    </div>
  );
}
