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

import { render, screen, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { LiveRun } from '../live-run-store';
import { SingleAgentLiveCard } from '../SingleAgentLiveCard';

import type { TimelineItem } from '@/api/kernel-types';

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
    errorCategory: null,
    errorDetail: null,
    upgradeUrl: null,
    currentStage: null,
    ...overrides,
  };
}

function toolUse(seq: number, tool: string, input: Record<string, unknown>): TimelineItem {
  return { seq, turn: 0, kind: 'tool_use', tool, input, streaming: true };
}

function toolResult(seq: number, tool: string, success: boolean, output?: string): TimelineItem {
  return { seq, turn: 0, kind: 'tool_result', tool, success, output };
}

function errorItem(seq: number, content: string): TimelineItem {
  return { seq, turn: 0, kind: 'error', content };
}

describe('SingleAgentLiveCard', () => {
  it('shows a generic working placeholder when the run has no items or stage', () => {
    render(<SingleAgentLiveCard run={runFixture()} onOpenTranscript={vi.fn()} />);
    expect(screen.getByText('正在处理…')).toBeInTheDocument();
  });

  it('renders the current stage text when set and no items', () => {
    render(
      <SingleAgentLiveCard
        run={runFixture({ currentStage: 'Waiting for LLM response (iteration 2)...' })}
        onOpenTranscript={vi.fn()}
      />,
    );
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

  it('renders a running chip with tool name + preview while the use is unpaired', () => {
    render(
      <SingleAgentLiveCard
        run={runFixture({
          items: [toolUse(1, 'grep', { query: 'fn main' })],
        })}
        onOpenTranscript={vi.fn()}
      />,
    );
    expect(screen.getByText('grep')).toBeInTheDocument();
    expect(screen.getByText('fn main')).toBeInTheDocument();
    expect(screen.getByRole('status', { name: 'running' })).toBeInTheDocument();
  });

  it('shows a completed icon once a successful tool_result is paired', () => {
    render(
      <SingleAgentLiveCard
        run={runFixture({
          items: [
            toolUse(1, 'grep', { query: 'fn main' }),
            toolResult(2, 'grep', true, 'match.rs:10'),
          ],
        })}
        onOpenTranscript={vi.fn()}
      />,
    );
    expect(screen.getByLabelText('completed')).toBeInTheDocument();
  });

  it('shows an errored icon when tool_result.success is false', () => {
    render(
      <SingleAgentLiveCard
        run={runFixture({
          items: [
            toolUse(1, 'grep', { query: 'fn main' }),
            toolResult(2, 'grep', false, 'permission denied'),
          ],
        })}
        onOpenTranscript={vi.fn()}
      />,
    );
    expect(screen.getByLabelText('errored')).toBeInTheDocument();
    expect(screen.getByText('permission denied')).toBeInTheDocument();
  });

  it('shows an errored icon with error text when paired with an error item', () => {
    render(
      <SingleAgentLiveCard
        run={runFixture({
          items: [toolUse(1, 'grep', { query: 'fn main' }), errorItem(2, 'kernel crashed')],
        })}
        onOpenTranscript={vi.fn()}
      />,
    );
    expect(screen.getByLabelText('errored')).toBeInTheDocument();
    expect(screen.getByText('kernel crashed')).toBeInTheDocument();
  });

  it('renders newest tool_use chip first', () => {
    const { container } = render(
      <SingleAgentLiveCard
        run={runFixture({
          items: [
            toolUse(1, 'grep', { query: 'older' }),
            toolUse(2, 'read', { file_path: '/etc/hosts' }),
          ],
        })}
        onOpenTranscript={vi.fn()}
      />,
    );
    const names = Array.from(container.querySelectorAll('.font-mono')).map(
      (n) => within(n as HTMLElement).queryByText(/./)?.textContent ?? n.textContent,
    );
    expect(names[0]).toBe('read');
    expect(names[1]).toBe('grep');
  });
});
