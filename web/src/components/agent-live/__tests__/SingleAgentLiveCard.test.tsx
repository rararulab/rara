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

import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { LiveRun } from '../live-run-store';
import { SingleAgentLiveCard } from '../SingleAgentLiveCard';

function runFixture(overrides: Partial<LiveRun> = {}): LiveRun {
  return {
    runId: 'r1',
    sessionKey: 's',
    status: 'running',
    startedAt: Date.now() - 5_000,
    endedAt: null,
    items: [],
    toolCalls: 0,
    error: null,
    ...overrides,
  };
}

describe('SingleAgentLiveCard', () => {
  it('shows the empty-log message when the run has no items yet', () => {
    render(<SingleAgentLiveCard run={runFixture()} onOpenTranscript={vi.fn()} />);
    expect(
      screen.getByText(
        /Live log is not available for this agent provider\. Results will appear when the task completes\./,
      ),
    ).toBeInTheDocument();
  });
});
