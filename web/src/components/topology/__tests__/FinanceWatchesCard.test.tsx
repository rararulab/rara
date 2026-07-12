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

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { FinanceWatchesCard } from '../FinanceWatchesCard';

import type { FeedCatalogEntry, FinanceSubscriptionsResponse } from '@/api/data-feeds';

const catalogMock = vi.fn();
const financeSubscriptionsMock = vi.fn();
const enableCatalogEntryMock = vi.fn();
const createFinanceSubscriptionMock = vi.fn();
const unsubscribeCatalogEntryMock = vi.fn();
const openSettingsMock = vi.fn();

vi.mock('@/api/data-feeds', () => ({
  dataFeedsApi: {
    catalog: (...args: unknown[]) => catalogMock(...args),
    financeSubscriptions: (...args: unknown[]) => financeSubscriptionsMock(...args),
    enableCatalogEntry: (...args: unknown[]) => enableCatalogEntryMock(...args),
    createFinanceSubscription: (...args: unknown[]) => createFinanceSubscriptionMock(...args),
    unsubscribeCatalogEntry: (...args: unknown[]) => unsubscribeCatalogEntryMock(...args),
  },
}));

vi.mock('@/components/settings/SettingsModalContext', () => ({
  useSettingsModal: () => ({
    openSettings: openSettingsMock,
    closeSettings: vi.fn(),
  }),
}));

function buildClient() {
  return new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: 0, staleTime: 0 },
      mutations: { retry: false },
    },
  });
}

function renderCard(sessionKey = 'session-1') {
  const client = buildClient();
  const utils = render(
    <QueryClientProvider client={client}>
      <FinanceWatchesCard sessionKey={sessionKey} />
    </QueryClientProvider>,
  );
  return { ...utils, client };
}

function catalogEntry(partial: Partial<FeedCatalogEntry> & { id: string }): FeedCatalogEntry {
  const entry: FeedCatalogEntry = {
    id: partial.id,
    name: partial.name ?? partial.id,
    description: partial.description ?? 'source description',
    feed_type: partial.feed_type ?? 'rss',
    tags: partial.tags ?? ['finance', 'news'],
    enabled: partial.enabled ?? true,
    feed_id: partial.feed_id ?? null,
    requires_configuration: partial.requires_configuration ?? false,
    setup_hint: partial.setup_hint ?? null,
    transport_template: partial.transport_template ?? null,
  };
  if (partial.source_name !== undefined) entry.source_name = partial.source_name;
  if (partial.venue !== undefined) entry.venue = partial.venue;
  if (partial.configured_symbols !== undefined)
    entry.configured_symbols = partial.configured_symbols;
  if (partial.configured_timeframes !== undefined) {
    entry.configured_timeframes = partial.configured_timeframes;
  }
  if (partial.subscriptions !== undefined) entry.subscriptions = partial.subscriptions;
  return entry;
}

beforeEach(() => {
  catalogMock.mockReset();
  financeSubscriptionsMock.mockReset();
  enableCatalogEntryMock.mockReset();
  createFinanceSubscriptionMock.mockReset();
  unsubscribeCatalogEntryMock.mockReset();
  openSettingsMock.mockReset();
});

afterEach(() => {
  cleanup();
});

describe('FinanceWatchesCard', () => {
  it('creates_session_watch_from_catalog_kline_source_with_configured_selectors', async () => {
    const binance = catalogEntry({
      id: 'binance-market-candles',
      name: 'Binance Market Candles',
      feed_type: 'market_candle',
      source_name: 'finance-binance-market-candles',
      venue: 'binance',
      configured_symbols: ['BTCUSDT', 'ETHUSDT'],
      configured_timeframes: ['1m'],
      tags: ['finance', 'market-data', 'crypto', 'binance'],
      transport_template: {
        venue: 'binance',
        symbols: ['BTCUSDT', 'ETHUSDT'],
        timeframes: ['1m'],
      },
    });
    catalogMock.mockResolvedValue([binance]);
    financeSubscriptionsMock.mockResolvedValue({
      subscriptions: [],
      count: 0,
    } satisfies FinanceSubscriptionsResponse);
    createFinanceSubscriptionMock.mockResolvedValue({
      created: true,
      subscription: {
        subscription_id: 'sub-1',
        session_key: 'session-1',
        event_kinds: ['market_candle_closed'],
        source_names: ['finance-binance-market-candles'],
        matches_all_sources: false,
        sources: [],
        category_tags: [],
        watch_terms: [],
        venues: ['binance'],
        symbols: ['BTCUSDT', 'ETHUSDT'],
        timeframes: ['1m'],
        delivery: 'silent',
        cooldown_secs: 900,
        max_immediate_per_hour: 6,
      },
    });

    renderCard('session-1');

    const button = await screen.findByRole('button', { name: 'Watch' });
    fireEvent.click(button);

    await waitFor(() => {
      expect(createFinanceSubscriptionMock).toHaveBeenCalledWith({
        session_key: 'session-1',
        catalog_source_ids: ['binance-market-candles'],
        delivery: 'silent',
        venues: ['binance'],
        symbols: ['BTCUSDT', 'ETHUSDT'],
        timeframes: ['1m'],
      });
    });
  });

  it('enables_ready_source_before_allowing_session_watch', async () => {
    catalogMock.mockResolvedValue([
      catalogEntry({
        id: 'fed-press-releases',
        name: 'Federal Reserve Press Releases',
        source_name: 'finance-fed-press-releases',
        enabled: false,
      }),
    ]);
    financeSubscriptionsMock.mockResolvedValue({
      subscriptions: [],
      count: 0,
    } satisfies FinanceSubscriptionsResponse);
    enableCatalogEntryMock.mockResolvedValue({
      id: 'feed-1',
      name: 'finance-fed-press-releases',
      feed_type: 'rss',
      tags: ['finance', 'news'],
      transport: {},
      auth: null,
      enabled: true,
      status: 'idle',
      last_error: null,
      created_at: '2026-01-01T00:00:00Z',
      updated_at: '2026-01-01T00:00:00Z',
    });

    renderCard('session-1');

    fireEvent.click(await screen.findByRole('button', { name: 'Enable source' }));

    await waitFor(() => {
      expect(enableCatalogEntryMock).toHaveBeenCalledWith('fed-press-releases');
    });
    expect(createFinanceSubscriptionMock).not.toHaveBeenCalled();
  });

  it('removes_existing_session_watch_by_subscription_id', async () => {
    catalogMock.mockResolvedValue([
      catalogEntry({
        id: 'fed-press-releases',
        name: 'Federal Reserve Press Releases',
        source_name: 'finance-fed-press-releases',
      }),
    ]);
    financeSubscriptionsMock.mockResolvedValue({
      count: 1,
      subscriptions: [
        {
          subscription_id: 'sub-1',
          session_key: 'session-1',
          event_kinds: ['rss_article'],
          source_names: ['finance-fed-press-releases'],
          matches_all_sources: false,
          sources: [],
          category_tags: [],
          watch_terms: [],
          venues: [],
          symbols: [],
          timeframes: [],
          delivery: 'silent',
          cooldown_secs: 900,
          max_immediate_per_hour: 6,
        },
      ],
    } satisfies FinanceSubscriptionsResponse);
    unsubscribeCatalogEntryMock.mockResolvedValue({
      catalog_source_id: 'fed-press-releases',
      source_name: 'finance-fed-press-releases',
      removed_subscription_ids: ['sub-1'],
      removed_count: 1,
      remaining_subscription_ids: [],
    });

    renderCard('session-1');

    expect(await screen.findByText('watching')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Unwatch' }));

    await waitFor(() => {
      expect(unsubscribeCatalogEntryMock).toHaveBeenCalledWith('fed-press-releases', {
        subscription_ids: ['sub-1'],
      });
    });
  });

  it('opens_data_feed_settings_for_sources_that_require_configuration', async () => {
    catalogMock.mockResolvedValue([
      catalogEntry({
        id: 'longport-market-candles',
        name: 'LongPort Market Candles',
        feed_type: 'market_candle',
        enabled: false,
        requires_configuration: true,
        setup_hint: 'Configure LongPort credentials and selectors before enabling.',
      }),
    ]);
    financeSubscriptionsMock.mockResolvedValue({
      subscriptions: [],
      count: 0,
    } satisfies FinanceSubscriptionsResponse);

    renderCard('session-1');

    fireEvent.click(await screen.findByRole('button', { name: 'Configure' }));

    expect(openSettingsMock).toHaveBeenCalledWith('data-feeds', {
      dataFeedCatalogId: 'longport-market-candles',
    });
    expect(createFinanceSubscriptionMock).not.toHaveBeenCalled();
    expect(enableCatalogEntryMock).not.toHaveBeenCalled();
  });
});
