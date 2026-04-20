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
import { useLiveRun } from './use-live-run';

interface Props {
  sessionKey: string | undefined;
}

/**
 * Inline in-progress card for an active run — styled and positioned to
 * behave like the next assistant message at the bottom of the chat
 * transcript (ChatGPT/Claude pattern). Parent owns the slot layout
 * (see `.rara-live-slot` in `index.css`); this component renders the
 * card content + transcript modal only.
 *
 * Task history has moved off this card — transient run history adds
 * noise in the message flow; surface it from the session sidebar
 * instead (follow-up).
 *
 * Stop is intentionally wired as `disabled` — the backend cancel
 * endpoint is not implemented yet (tracked in a follow-up issue).
 */
export function AgentLiveCard({ sessionKey }: Props) {
  const slice = useLiveRun(sessionKey);
  const [openRun, setOpenRun] = useState<LiveRun | null>(null);

  if (!slice.active) return null;

  return (
    <>
      <SingleAgentLiveCard
        run={slice.active}
        onOpenTranscript={() => setOpenRun(slice.active)}
        // Stop endpoint not yet wired — see follow-up issue referenced
        // in the PR body. Passing no handler disables the button with
        // a clarifying tooltip inside SingleAgentLiveCard.
        {...({} as { onStop?: () => void })}
      />
      <AgentTranscriptDialog
        run={openRun}
        open={openRun !== null}
        onClose={() => setOpenRun(null)}
      />
    </>
  );
}
