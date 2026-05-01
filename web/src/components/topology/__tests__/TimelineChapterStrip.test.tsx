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
 * BDD bindings for the frontend half of
 * `specs/issue-2040-anchor-segment-chat-history.spec.md`.
 *
 * Each `it(...)` name carries the spec's `Filter:` selector verbatim so
 * a human (or the eventual vitest adapter for `agent-spec`) can resolve
 * scenarios to real test functions. The strip is a pure component
 * driven by props, so the tests mount it directly with stubbed
 * `onSelectAnchor` and assert on rendered DOM + callback shape.
 */

import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { TimelineChapterStrip } from '../TimelineChapterStrip';

import type { SessionAnchor } from '@/api/types';

afterEach(() => {
  cleanup();
});

function anchor(id: number, name: string, byteOffset: number, entryCount: number): SessionAnchor {
  return {
    anchor_id: id,
    byte_offset: byteOffset,
    name,
    timestamp: '2026-04-30T00:00:00Z',
    entry_count_in_segment: entryCount,
  };
}

describe('TimelineChapterStrip', () => {
  it('renders_marker_per_anchor', () => {
    const anchors = [anchor(1, 'N1', 0, 3), anchor(2, 'N2', 100, 5), anchor(3, 'N3', 250, 7)];
    render(<TimelineChapterStrip anchors={anchors} onSelectAnchor={() => {}} />);

    const markers = screen.getAllByTestId('chapter-marker');
    expect(markers).toHaveLength(3);

    // Order matches the input array.
    expect(markers[0]).toHaveTextContent('N1');
    expect(markers[1]).toHaveTextContent('N2');
    expect(markers[2]).toHaveTextContent('N3');

    // Each marker exposes its `entry_count_in_segment` as a badge.
    const badges = screen.getAllByTestId('chapter-marker-count');
    expect(badges.map((b) => b.textContent)).toEqual(['3', '5', '7']);

    // Long names truncate in the visible label but the full name lives
    // on `title=""` so hover still surfaces it. We assert the truncation
    // contract here so a regression that drops the title attr would
    // break the test.
    cleanup();
    const longName = 'daily-summary-2026-04-28';
    render(
      <TimelineChapterStrip anchors={[anchor(1, longName, 0, 1)]} onSelectAnchor={() => {}} />,
    );
    const marker = screen.getByTestId('chapter-marker');
    expect(marker).toHaveAttribute('title', longName);
    expect(marker.textContent).not.toContain('daily-summary-2026-04-28');
    expect(marker.textContent).toContain('…');
  });

  it('click_marker_fetches_and_scrolls', () => {
    // Spec scenario 9 has two halves: (1) the click invokes the parent
    // with the right `(from, to)` pair so the parent can issue
    // `from_anchor=A2 & to_anchor=A3`, and (2) the parent threads the
    // returned messages back into TimelineView which scrolls. The
    // strip's contract is exclusively half (1) — half (2) lives in
    // `TimelineView`'s own `segmentMessages`-driven scroll effect (set
    // in this PR; covered by manual smoke). The pure-component split
    // is intentional: a regression in click→callback wiring is what
    // would silently break the navigation.
    const onSelect = vi.fn();
    const a1 = anchor(1, 'A1', 0, 2);
    const a2 = anchor(2, 'A2', 100, 4);
    const a3 = anchor(3, 'A3', 220, 6);

    render(<TimelineChapterStrip anchors={[a1, a2, a3]} onSelectAnchor={onSelect} />);

    const markers = screen.getAllByTestId('chapter-marker');
    fireEvent.click(markers[1]!); // A2

    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(onSelect).toHaveBeenCalledWith(a2, a3);
  });

  it('most_recent_marker_omits_to_anchor', () => {
    // A3 is the most recent (last in the array). Clicking it must call
    // the parent with `to = null`, which the API helper translates to
    // "no `to_anchor` query param" → backend reads to EOF.
    const onSelect = vi.fn();
    const a1 = anchor(1, 'A1', 0, 2);
    const a2 = anchor(2, 'A2', 100, 4);
    const a3 = anchor(3, 'A3', 220, 6);

    render(<TimelineChapterStrip anchors={[a1, a2, a3]} onSelectAnchor={onSelect} />);

    const markers = screen.getAllByTestId('chapter-marker');
    fireEvent.click(markers[2]!); // A3 (most recent)

    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(onSelect).toHaveBeenCalledWith(a3, null);
  });
});
