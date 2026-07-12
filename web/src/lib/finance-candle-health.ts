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
