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
    currentStage: null,
    ...overrides,
  };
}

describe('SingleAgentLiveCard', () => {
  it('shows a generic working placeholder when the run has no items or stage', () => {
    render(<SingleAgentLiveCard run={runFixture()} onOpenTranscript={vi.fn()} />);
    expect(screen.getByText('正在处理…')).toBeInTheDocument();
  });

  it('renders the current stage text when set', () => {
    render(
      <SingleAgentLiveCard
        run={runFixture({ currentStage: 'Waiting for LLM response (iteration 2)...' })}
        onOpenTranscript={vi.fn()}
      />,
    );
    // Appears both in header subtitle and body row.
    expect(
      screen.getAllByText(/Waiting for LLM response \(iteration 2\)\.\.\./).length,
    ).toBeGreaterThan(0);
  });

  it('beautifies the well-known "thinking" stage', () => {
    render(
      <SingleAgentLiveCard
        run={runFixture({ currentStage: 'thinking' })}
        onOpenTranscript={vi.fn()}
      />,
    );
    expect(screen.getAllByText('思考中…').length).toBeGreaterThan(0);
  });
});
