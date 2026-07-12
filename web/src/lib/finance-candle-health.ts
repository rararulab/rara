/*
 * Copyright 2026 Rararulab
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

import type { CandleStream, FinanceSubscription } from '@/api/data-feeds';

export type CandleStreamHealth = {
  status: 'fresh' | 'stale' | 'missing';
  label: string;
  detail: string;
};

export type CandleStreamSelectors = {
  sourceNames?: string[];
  matchesAllSources?: boolean;
  venues?: string[];
  symbols?: string[];
  timeframes?: string[];
};

export type ExpectedCandleStreamSelectors = {
  sourceName: string;
  venue?: string | null;
  symbols: string[];
  timeframes: string[];
};

export function candleSubscriptionStreamHealth(
  subscription: FinanceSubscription,
  streams: CandleStream[],
): CandleStreamHealth | null {
  if (!subscription.event_kinds.includes('market_candle_closed')) return null;

  return candleStreamHealthForSelectors(
    {
      sourceNames: subscription.source_names,
      matchesAllSources: subscription.matches_all_sources,
      venues: subscription.venues,
      symbols: subscription.symbols,
      timeframes: subscription.timeframes,
    },
    streams,
  );
}

export function candleStreamHealthForSelectors(
  selectors: CandleStreamSelectors,
  streams: CandleStream[],
): CandleStreamHealth {
  const matchingStreams = streams.filter((stream) => streamMatchesSelectors(stream, selectors));
  if (matchingStreams.length === 0) {
    return {
      status: 'missing',
      label: 'Missing',
      detail: 'No stored K-line stream matches this subscription yet.',
    };
  }

  const staleCount = matchingStreams.filter(isStaleStream).length;
  if (staleCount === matchingStreams.length) {
    return {
      status: 'stale',
      label: 'Stale',
      detail: `${staleCount} matched stream${staleCount === 1 ? '' : 's'} past the freshness window.`,
    };
  }

  const freshCount = matchingStreams.length - staleCount;
  return {
    status: 'fresh',
    label: 'Fresh',
    detail: `${freshCount}/${matchingStreams.length} matched stream${matchingStreams.length === 1 ? '' : 's'} fresh.`,
  };
}

export function expectedCandleStreamHealth(
  selectors: ExpectedCandleStreamSelectors,
  streams: CandleStream[],
): CandleStreamHealth {
  const symbols = Array.from(normalizedSelectorSet(selectors.symbols, 'upper'));
  const timeframes = Array.from(normalizedSelectorSet(selectors.timeframes, 'timeframe'));
  const sourceName = selectors.sourceName.trim().toLowerCase();
  const venue = selectors.venue?.trim().toLowerCase() || null;

  if (!sourceName || symbols.length === 0 || timeframes.length === 0) {
    return candleStreamHealthForSelectors(
      {
        sourceNames: sourceName ? [sourceName] : [],
        venues: venue ? [venue] : [],
        symbols,
        timeframes,
      },
      streams,
    );
  }

  const expected = symbols.flatMap((symbol) =>
    timeframes.map((timeframe) => ({
      sourceName,
      venue,
      symbol,
      timeframe,
    })),
  );
  const matchedStreams: CandleStream[] = [];
  const missing = expected.filter((selector) => {
    const stream = streams.find((candidate) => expectedSelectorMatchesStream(selector, candidate));
    if (stream) {
      matchedStreams.push(stream);
      return false;
    }
    return true;
  });

  if (missing.length > 0) {
    const presentCount = expected.length - missing.length;
    if (presentCount === 0) {
      return {
        status: 'missing',
        label: 'Missing',
        detail: 'No stored K-line stream matches this subscription yet.',
      };
    }

    return {
      status: 'missing',
      label: 'Partial',
      detail: `${presentCount}/${expected.length} expected stream${expected.length === 1 ? '' : 's'} present; ${missing.length} missing.`,
    };
  }

  const staleCount = matchedStreams.filter(isStaleStream).length;
  if (staleCount === matchedStreams.length) {
    return {
      status: 'stale',
      label: 'Stale',
      detail: `${staleCount} expected stream${staleCount === 1 ? '' : 's'} past the freshness window.`,
    };
  }

  if (staleCount > 0) {
    const freshCount = matchedStreams.length - staleCount;
    return {
      status: 'stale',
      label: 'Partial',
      detail: `${freshCount}/${expected.length} expected stream${expected.length === 1 ? '' : 's'} fresh; ${staleCount} stale.`,
    };
  }

  return {
    status: 'fresh',
    label: 'Fresh',
    detail: `${expected.length}/${expected.length} expected stream${expected.length === 1 ? '' : 's'} fresh.`,
  };
}

function streamMatchesSelectors(stream: CandleStream, selectors: CandleStreamSelectors): boolean {
  const sourceNames = normalizedSelectorSet(selectors.sourceNames ?? [], 'lower');
  const venues = normalizedSelectorSet(selectors.venues ?? [], 'lower');
  const symbols = normalizedSelectorSet(selectors.symbols ?? [], 'upper');
  const timeframes = normalizedSelectorSet(selectors.timeframes ?? [], 'timeframe');

  if (
    sourceNames.size > 0 &&
    !selectors.matchesAllSources &&
    !sourceNames.has(stream.source_name.toLowerCase())
  ) {
    return false;
  }
  if (venues.size > 0 && !venues.has(stream.venue.toLowerCase())) return false;
  if (symbols.size > 0 && !symbols.has(stream.symbol.toUpperCase())) return false;
  if (timeframes.size > 0 && !timeframes.has(stream.timeframe.toLowerCase())) return false;
  return true;
}

function expectedSelectorMatchesStream(
  selector: { sourceName: string; venue: string | null; symbol: string; timeframe: string },
  stream: CandleStream,
): boolean {
  if (stream.source_name.toLowerCase() !== selector.sourceName) return false;
  if (selector.venue && stream.venue.toLowerCase() !== selector.venue) return false;
  if (stream.symbol.toUpperCase() !== selector.symbol) return false;
  return stream.timeframe.toLowerCase() === selector.timeframe;
}

function normalizedSelectorSet(
  values: string[],
  mode: 'lower' | 'upper' | 'timeframe',
): Set<string> {
  return new Set(
    values
      .map((value) => value.trim())
      .filter(Boolean)
      .map((value) => {
        if (mode === 'lower') return value.toLowerCase();
        if (mode === 'upper') return value.toUpperCase();
        return value.toLowerCase();
      }),
  );
}

function isStaleStream(stream: CandleStream): boolean {
  const stepSeconds = timeframeSeconds(stream.timeframe);
  if (stepSeconds == null) return false;
  const latestCloseMs = new Date(stream.latest_close_time).getTime();
  if (!Number.isFinite(latestCloseMs)) return false;
  const lagSeconds = Math.floor((Date.now() - latestCloseMs) / 1000);
  return lagSeconds > stepSeconds * 2;
}

function timeframeSeconds(timeframe: string): number | null {
  const match = /^(\d+)([smhd])$/i.exec(timeframe.trim());
  if (!match) return null;
  const [, amountText, unit] = match;
  if (!amountText || !unit) return null;
  const amount = Number(amountText);
  if (!Number.isFinite(amount) || amount <= 0) return null;
  switch (unit.toLowerCase()) {
    case 's':
      return amount;
    case 'm':
      return amount * 60;
    case 'h':
      return amount * 60 * 60;
    case 'd':
      return amount * 24 * 60 * 60;
    default:
      return null;
  }
}
