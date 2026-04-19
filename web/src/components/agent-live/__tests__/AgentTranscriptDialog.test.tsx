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

import { AgentTranscriptDialog } from '../AgentTranscriptDialog';
import type { LiveRun } from '../live-run-store';

function runFixture(): LiveRun {
  return {
    runId: 'r1',
    sessionKey: 's',
    status: 'completed',
    startedAt: Date.UTC(2026, 3, 1, 12, 0, 0),
    endedAt: Date.UTC(2026, 3, 1, 12, 0, 7),
    items: [
      { seq: 0, turn: 0, kind: 'thinking', content: 'planning the work' },
      { seq: 1, turn: 0, kind: 'tool_use', tool: 'Grep', input: { query: 'needle' } },
    ],
    toolCalls: 1,
    error: null,
  };
}

describe('AgentTranscriptDialog', () => {
  it('renders nothing when run is null', () => {
    const { container } = render(
      <AgentTranscriptDialog run={null} open={true} onClose={vi.fn()} />,
    );
    expect(container.firstChild).toBeNull();
  });

  it('renders the header metadata and closes when the Close button is clicked', () => {
    const onClose = vi.fn();
    render(<AgentTranscriptDialog run={runFixture()} open={true} onClose={onClose} />);

    // Header counts.
    expect(screen.getByText(/1 tool calls/)).toBeInTheDocument();
    expect(screen.getByText(/2 events/)).toBeInTheDocument();
    expect(screen.getByText('Completed')).toBeInTheDocument();

    fireEvent.click(screen.getByLabelText('Close transcript'));
    expect(onClose).toHaveBeenCalled();
  });
});
