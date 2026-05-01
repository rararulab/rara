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

/**
 * BDD bindings for `specs/issue-2023-topology-trace-cascade-buttons.spec.md`.
 *
 * Each `it(...)` carries the spec's `Filter:` selector verbatim so
 * `agent-spec verify` can resolve scenarios to real test functions once
 * the vitest adapter (issue 2015) lands.
 */

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { RaraTurnCard } from '../RaraTurnCard';
import type { TurnCardData } from '../TurnCard';

import { ApiError } from '@/api/client';

import '@/i18n';

// Mock the fetch wrappers so the tests exercise the wiring, not the network.
const fetchExecutionTraceMock = vi.fn();
const fetchCascadeTraceMock = vi.fn();
vi.mock('@/api/sessions', () => ({
  fetchExecutionTrace: (...args: unknown[]) => fetchExecutionTraceMock(...args),
  fetchCascadeTrace: (...args: unknown[]) => fetchCascadeTraceMock(...args),
}));

function makeTurn(overrides: Partial<TurnCardData> = {}): TurnCardData {
  return {
    id: 'turn-42',
    text: 'final assistant text',
    reasoning: '',
    toolCalls: [
      {
        id: 'tool-1',
        name: 'shell',
        result: { success: true, preview: 'ok', error: null },
      },
    ],
    markers: [],
    metrics: null,
    usage: null,
    inFlight: false,
    createdAt: 1_700_000_000_000,
    finalSeq: 42,
    ...overrides,
  };
}

function renderCard(turn: TurnCardData, sessionKey = 'sess-abc') {
  // Each test gets a fresh QueryClient so cached results from earlier
  // scenarios cannot leak into the next one.
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0, staleTime: 0 } },
  });
  return render(
    <QueryClientProvider client={qc}>
      <RaraTurnCard turn={turn} sessionKey={sessionKey} />
    </QueryClientProvider>,
  );
}

/**
 * Locate the vendor's three-dot actions trigger. The vendor renders it
 * as `<div role="button">` containing a lucide `MoreHorizontal` SVG with
 * `class="lucide-more-horizontal"`. Returning the outer div lets the
 * test fire a real click.
 */
function findActionsTrigger(): HTMLElement | null {
  // Lucide renamed `MoreHorizontal` → `Ellipsis` while keeping the export
  // name; the rendered icon class is now `lucide-ellipsis`. Match either
  // so the test does not break on a vendor-side lucide bump.
  const icon =
    document.querySelector('.lucide-ellipsis') || document.querySelector('.lucide-more-horizontal');
  if (!icon) return null;
  return icon.closest('[role="button"]');
}

afterEach(() => {
  cleanup();
  fetchExecutionTraceMock.mockReset();
  fetchCascadeTraceMock.mockReset();
});

describe('RaraTurnCard — trace + cascade affordances', () => {
  beforeEach(() => {
    fetchExecutionTraceMock.mockReset();
    fetchCascadeTraceMock.mockReset();
  });

  it('RaraTurnCard__actions_menu_wired_when_finalSeq_present_and_not_inflight', () => {
    renderCard(makeTurn({ finalSeq: 42, inFlight: false }));
    const trigger = findActionsTrigger();
    expect(trigger).not.toBeNull();

    fireEvent.click(trigger as HTMLElement);
    // The dropdown is rendered to a portal — `screen` queries the whole
    // document, which covers it.
    expect(screen.getByText(/view turn details/i)).toBeInTheDocument();
  });

  it('RaraTurnCard__actions_menu_suppressed_when_inflight_or_seq_null', () => {
    renderCard(makeTurn({ finalSeq: null, inFlight: true }));
    expect(findActionsTrigger()).toBeNull();
    expect(screen.queryByText(/view turn details/i)).toBeNull();
  });

  it('RaraTurnCard__trace_modal_opens_with_fetched_content', async () => {
    fetchExecutionTraceMock.mockResolvedValue({
      duration_secs: 1.23,
      iterations: 7,
      model: 'gpt-fixture-v9',
      input_tokens: 100,
      output_tokens: 200,
      thinking_ms: 50,
      thinking_preview: '',
      plan_steps: [],
      tools: [],
      rara_turn_id: 'turn-42',
    });

    renderCard(makeTurn({ finalSeq: 42, inFlight: false }));
    const trigger = findActionsTrigger();
    expect(trigger).not.toBeNull();
    fireEvent.click(trigger as HTMLElement);
    fireEvent.click(screen.getByText(/view turn details/i));

    // The modal renders a `dialog` role; it should appear with the fetched
    // model name and iteration count from the fixture.
    const dialog = await screen.findByRole('dialog');
    await waitFor(() => {
      expect(within(dialog).getByText('gpt-fixture-v9')).toBeInTheDocument();
    });
    expect(within(dialog).getByText('7')).toBeInTheDocument();
    expect(fetchExecutionTraceMock).toHaveBeenCalledWith(
      'sess-abc',
      42,
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
  });

  it('RaraTurnCard__cascade_modal_opens_from_activity_row', async () => {
    fetchCascadeTraceMock.mockResolvedValue({
      message_id: 'sess-abc-42',
      ticks: [
        {
          index: 0,
          entries: [
            {
              id: 'cascade.0-aaa-1',
              kind: 'thought',
              content: 'cascade-fixture-thought',
              timestamp: '2026-04-30T00:00:00Z',
            },
          ],
        },
      ],
      summary: { tick_count: 1, tool_call_count: 1, total_entries: 1 },
    });

    renderCard(makeTurn({ finalSeq: 42, inFlight: false }));
    // Vendor renders activities lazily inside an expandable section —
    // expand the turn first by clicking the chevron header. The chevron
    // icon is wrapped inside the toggle button.
    const chevron = document.querySelector('.lucide-chevron-right');
    expect(chevron).not.toBeNull();
    const toggleBtn = (chevron as Element).closest('button');
    expect(toggleBtn).not.toBeNull();
    fireEvent.click(toggleBtn as HTMLElement);

    // Now the tool activity row is in the DOM. Clicking the row container
    // (which has onClick={onOpenDetails && isComplete ? onOpenDetails : …})
    // fires onOpenActivityDetails. We locate the row via the tool name.
    const toolNameNode = await screen.findByText('shell');
    // The clickable container is the row `div` (group/row); walk up until
    // we find a node with an onClick handler. `closest('div.group\\/row')`
    // is brittle because Tailwind class names vary; instead, fire on the
    // text node's parent — React's synthetic event bubbles up to the row
    // handler regardless.
    fireEvent.click(toolNameNode);

    const dialog = await screen.findByRole('dialog');
    await waitFor(() => {
      expect(within(dialog).getByText(/cascade-fixture-thought/)).toBeInTheDocument();
    });
    expect(fetchCascadeTraceMock).toHaveBeenCalledWith(
      'sess-abc',
      42,
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
  });

  // BDD bindings for `specs/issue-2031-thinking-only-turn-render.spec.md`.
  // The vendor `TurnCard` suppresses turns whose only activity is
  // `type: 'thinking'` (`hasNoMeaningfulWork` gate); the adapter reroutes
  // thinking-only turns to `type: 'intermediate'` so the card renders.

  it('thinking_only_turn_renders_card', async () => {
    renderCard(
      makeTurn({
        text: '',
        reasoning: 'let me think about this',
        toolCalls: [],
        inFlight: false,
      }),
    );
    const wrapper = document.querySelector('[data-turn-id="turn-42"]');
    expect(wrapper).not.toBeNull();
    // Activities are collapsed by default; expand to expose the reasoning
    // row so the test can assert the reasoning content reached the DOM.
    const chevron = document.querySelector('.lucide-chevron-right');
    expect(chevron).not.toBeNull();
    fireEvent.click((chevron as Element).closest('button') as HTMLElement);
    await waitFor(() => {
      expect(screen.getByText(/let me think about this/)).toBeInTheDocument();
    });
  });

  it('thinking_only_live_turn_renders_card', () => {
    renderCard(
      makeTurn({
        text: '',
        reasoning: 'working it out',
        toolCalls: [],
        inFlight: true,
        finalSeq: null,
      }),
    );
    const wrapper = document.querySelector('[data-turn-id="turn-42"]');
    expect(wrapper).not.toBeNull();
  });

  it('turn_with_reasoning_and_text_renders_both', async () => {
    renderCard(
      makeTurn({
        text: 'here is the answer',
        reasoning: 'step 1: ...',
        toolCalls: [],
        inFlight: false,
      }),
    );
    const wrapper = document.querySelector('[data-turn-id="turn-42"]');
    expect(wrapper).not.toBeNull();
    // Final assistant text is the response slot — visible without expansion.
    expect(screen.getByText('here is the answer')).toBeInTheDocument();
    // The reasoning trace surfaces as an activity row in the expanded
    // section. Reasoning + text keeps the distinct `thinking` visual
    // treatment per spec Decision 2; the activity row label
    // ("Thinking" or "Processing") proves the reasoning trace was not
    // dropped on the floor.
    const chevron = document.querySelector('.lucide-chevron-right');
    expect(chevron).not.toBeNull();
    fireEvent.click((chevron as Element).closest('button') as HTMLElement);
    await waitFor(() => {
      const expanded = document.querySelector('[data-search-exclude="true"]');
      expect(expanded?.textContent).toMatch(/Thinking|Processing/);
    });
  });

  it('empty_turn_remains_suppressed', () => {
    renderCard(
      makeTurn({
        text: '',
        reasoning: '',
        toolCalls: [],
        inFlight: false,
      }),
    );
    expect(document.querySelector('[data-turn-id="turn-42"]')).toBeNull();
  });


  // BDD: actions_menu_renders_in_dom
  // Falsifier: revert RaraTurnCardActionsMenu wiring; assertion goes
  // back to failing because vendor SimpleDropdown bails on mount.
  it('RaraTurnCard__actions_menu_renders_in_dom', () => {
    renderCard(makeTurn({ finalSeq: 22, inFlight: false }));
    const icons = document.querySelectorAll('svg.lucide-more-horizontal, svg.lucide-ellipsis');
    expect(icons.length).toBeGreaterThanOrEqual(1);
  });

  // BDD: cascade_modal_only_on_tool_rows
  // Falsifier: drop the `activity.type === 'tool'` guard; clicking the
  // thinking row opens the cascade modal and the first assertion fails.
  it('RaraTurnCard__cascade_modal_only_on_tool_rows', async () => {
    fetchCascadeTraceMock.mockResolvedValue({
      message_id: 'sess-abc-22',
      ticks: [],
      summary: { tick_count: 0, tool_call_count: 0, total_entries: 0 },
    });

    const turn = makeTurn({
      finalSeq: 22,
      inFlight: false,
      reasoning: 'thinking out loud',
      toolCalls: [
        {
          id: 'tool-bash-1',
          name: 'bash',
          result: { success: true, preview: 'ok', error: null },
        },
      ],
    });
    renderCard(turn);

    const chevron = document.querySelector('.lucide-chevron-right');
    const toggleBtn = chevron?.closest('button');
    expect(toggleBtn).not.toBeNull();
    fireEvent.click(toggleBtn as HTMLElement);

    // Click the thinking row — cascade modal must NOT open. The vendor
    // renders our synthetic `type: 'thinking'` activity through its
    // fallback row with the i18n label "Processing" (no toolName, no
    // displayName → `formatToolDisplay` returns the i18n
    // `turnCard.processing` string). The label is incidental; the
    // load-bearing assertion is that clicking it does not open the
    // cascade modal.
    const thinkingSpan = Array.from(document.querySelectorAll('span')).find(
      (s) => s.textContent?.trim() === 'Processing',
    );
    expect(thinkingSpan).toBeTruthy();
    fireEvent.click(thinkingSpan as HTMLElement);
    expect(screen.queryByRole('dialog')).toBeNull();

    // Click the tool row — cascade modal must open.
    const toolNode = await screen.findByText('bash');
    fireEvent.click(toolNode);
    expect(await screen.findByRole('dialog')).toBeInTheDocument();
  });

  // BDD: trace_modal_friendly_404
  // Falsifier: drop the isSeqDivergence404 branch; the modal renders
  // the raw "Failed to load trace: …" string and the friendly-copy
  // assertion fails.
  it('RaraTurnCard__trace_modal_friendly_404', async () => {
    fetchExecutionTraceMock.mockRejectedValue(
      new ApiError(404, 'user message at seq 22 has no rara_turn_id metadata'),
    );

    renderCard(makeTurn({ finalSeq: 22, inFlight: false }));
    const trigger = findActionsTrigger();
    expect(trigger).not.toBeNull();
    fireEvent.click(trigger as HTMLElement);
    fireEvent.click(screen.getByText(/view turn details/i));

    const dialog = await screen.findByRole('dialog');
    await waitFor(() => {
      expect(within(dialog).getByRole('alert')).toBeInTheDocument();
    });
    expect(
      within(dialog).getByText(/Trace data is not available for this turn yet/i),
    ).toBeInTheDocument();
    expect(within(dialog).queryByText(/rara_turn_id metadata/i)).toBeNull();
  });

  // BDD: trace_modal_non_404_error_distinct
  // Falsifier: route every error through the friendly-404 branch; the
  // distinct-copy assertion below fails because the raw "internal error"
  // is no longer rendered.
  it('RaraTurnCard__trace_modal_non_404_error_distinct', async () => {
    fetchExecutionTraceMock.mockRejectedValue(new ApiError(500, 'internal error'));

    renderCard(makeTurn({ finalSeq: 22, inFlight: false }));
    const trigger = findActionsTrigger();
    fireEvent.click(trigger as HTMLElement);
    fireEvent.click(screen.getByText(/view turn details/i));

    const dialog = await screen.findByRole('dialog');
    await waitFor(() => {
      expect(within(dialog).getByRole('alert')).toBeInTheDocument();
    });
    expect(within(dialog).getByText(/internal error/i)).toBeInTheDocument();
    expect(within(dialog).queryByText(/Trace data is not available for this turn yet/i)).toBeNull();
  });

  it('RaraTurnCard__trace_modal_shows_error_on_fetch_failure', async () => {
    fetchExecutionTraceMock.mockRejectedValue(new Error('network is down'));

    renderCard(makeTurn({ finalSeq: 42, inFlight: false }));
    const trigger = findActionsTrigger();
    expect(trigger).not.toBeNull();
    fireEvent.click(trigger as HTMLElement);
    fireEvent.click(screen.getByText(/view turn details/i));

    const dialog = await screen.findByRole('dialog');
    await waitFor(() => {
      expect(within(dialog).getByRole('alert')).toBeInTheDocument();
    });
    expect(within(dialog).getByText(/network is down/i)).toBeInTheDocument();
    // The card itself is still rendered alongside the modal.
    expect(screen.getByText('final assistant text')).toBeInTheDocument();
  });
});
