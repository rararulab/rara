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

import { render, screen, fireEvent } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { LiveRun } from '../live-run-store';
import { TaskRunHistory } from '../TaskRunHistory';

function runFixture(overrides: Partial<LiveRun> = {}): LiveRun {
  return {
    runId: 'r1',
    sessionKey: 's',
    status: 'completed',
    startedAt: Date.UTC(2026, 3, 1, 12, 0, 0),
    endedAt: Date.UTC(2026, 3, 1, 12, 0, 5),
    items: [
      { seq: 0, turn: 0, kind: 'tool_use', tool: 'Grep', input: { query: 'hello' } },
      { seq: 1, turn: 0, kind: 'tool_result', tool: 'Grep', output: '3 matches' },
    ],
    toolCalls: 1,
    error: null,
    errorCategory: null,
    errorDetail: null,
    upgradeUrl: null,
    currentStage: null,
    ...overrides,
  };
}

describe('TaskRunHistory', () => {
  it('returns nothing when runs is empty', () => {
    const { container } = render(<TaskRunHistory runs={[]} onOpenTranscript={vi.fn()} />);
    expect(container.firstChild).toBeNull();
  });

  it('expands a run to reveal its timeline rows', async () => {
    const run = runFixture();
    render(<TaskRunHistory runs={[run]} onOpenTranscript={vi.fn()} />);

    // History section itself starts collapsed; open it.
    fireEvent.click(screen.getByText('Execution history'));
    // Now the run row is visible; click its Expand button.
    fireEvent.click(screen.getByText('Expand'));

    // The tool summary "hello" should now be in the DOM (from Grep input).
    // TimelineRow shows the summary from eventSummary() which reads the
    // tool's `query` field.
    expect(screen.getByText('hello')).toBeInTheDocument();
  });

  it('invokes onOpenTranscript when the transcript button is clicked', () => {
    const run = runFixture();
    const open = vi.fn();
    render(<TaskRunHistory runs={[run]} onOpenTranscript={open} />);
    fireEvent.click(screen.getByText('Execution history'));
    fireEvent.click(screen.getByLabelText('Open full transcript'));
    expect(open).toHaveBeenCalledTimes(1);
    expect(open.mock.calls[0]?.[0]).toBe(run);
  });
});
