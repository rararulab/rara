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

import { api } from './client';

export const CANDLE_STREAM_OVERVIEW_LIMIT = 500;

// ---------------------------------------------------------------------------
// Types — mirrors kernel::data_feed::config
// ---------------------------------------------------------------------------

export interface DataFeedConfig {
  id: string;
  name: string;
  feed_type: FeedType;
  tags: string[];
  transport: Record<string, unknown>;
  auth: AuthConfig | null;
  enabled: boolean;
  status: 'idle' | 'running' | 'error';
  last_error: string | null;
  created_at: string;
  updated_at: string;
}

export type AuthType = 'header' | 'query' | 'bearer' | 'basic' | 'hmac';

export type FeedType = 'webhook' | 'websocket' | 'polling' | 'rss' | 'market_candle';

export interface AuthConfig {
  type: AuthType;
  [key: string]: unknown;
}

export interface FeedEvent {
  id: string;
  source_name: string;
  event_type: string;
  tags: string[];
  payload: unknown;
  received_at: string;
}

export interface FeedEventsResponse {
  events: FeedEvent[];
  total: number;
  has_more: boolean;
}

export interface FeedSummary {
  feed_id: string;
  source_name: string;
  event_count: number;
  last_event_type: string | null;
  last_event_at: string | null;
  lag_seconds: number | null;
}

export interface CandleStream {
  source_name: string;
  venue: string;
  symbol: string;
  timeframe: string;
  candle_count: number;
  first_open_time: string;
  latest_open_time: string;
  latest_close_time: string;
  latest_ingested_at: string;
}

export interface CandleStreamsResponse {
  streams: CandleStream[];
  count: number;
  query_limit: number;
  has_more: boolean;
}

export interface MarketCandle {
  source_name: string;
  venue: string;
  symbol: string;
  timeframe: string;
  open_time: string;
  close_time: string;
  open: string;
  high: string;
  low: string;
  close: string;
  volume: string;
  ingested_at: string;
  provider_sequence: string | null;
}

export interface LatestMarketCandleResponse {
  candle: MarketCandle | null;
}

export interface MarketCandlesResponse {
  candles: MarketCandle[];
  count: number;
  query_limit: number;
  has_more: boolean;
  next_start: string | null;
}

export interface RecentMarketCandlesResponse {
  candles: MarketCandle[];
  count: number;
  query_limit: number;
  has_more: boolean;
  next_end: string | null;
}

export interface MarketCandleFreshnessResponse {
  latest: MarketCandle | null;
  as_of: string;
  stale_after_secs: number;
  lag_secs: number | null;
  is_stale: boolean;
  status: 'missing' | 'future' | 'stale' | 'fresh';
}

export interface MarketCandleGapsResponse {
  missing_open_times: string[];
  missing_count: number;
  expected_count: number;
  complete: boolean;
}

export interface CreateFeedRequest {
  name: string;
  feed_type: FeedType;
  tags: string[];
  transport: Record<string, unknown>;
  auth: AuthConfig | null;
}

export interface FeedCatalogEntry {
  id: string;
  name: string;
  description: string;
  feed_type: FeedType;
  provider?: string | null;
  tags: string[];
  source_name?: string;
  enabled: boolean;
  feed_id: string | null;
  requires_configuration: boolean;
  setup_hint: string | null;
  transport_template: Record<string, unknown> | null;
  venue?: string | null;
  configured_symbols?: string[];
  configured_timeframes?: string[];
  subscriptions?: FeedCatalogSubscriptions;
}

export interface FeedCatalogSubscriptions {
  user_subscribed: boolean;
  user_subscription_ids: string[];
}

export interface FinanceFeedBundle {
  id: string;
  name: string;
  description: string;
  tags: string[];
  catalog_source_ids: string[];
  feed_types: FeedType[];
  providers: string[];
  source_count: number;
  enabled_source_count: number;
  ready_source_count: number;
  requires_configuration: boolean;
  can_enable: boolean;
  sources: FeedCatalogEntry[];
  subscriptions: FeedCatalogSubscriptions;
}

export interface FinanceFeedBundlesResponse {
  bundles: FinanceFeedBundle[];
  count: number;
}

export interface EnableCatalogEntryRequest {
  transport?: Record<string, unknown>;
  auth?: AuthConfig | null;
}

export interface UnsubscribeCatalogEntryRequest {
  subscription_ids?: string[];
}

export interface UnsubscribeCatalogEntryResponse {
  catalog_source_id: string;
  source_name: string;
  removed_subscription_ids: string[];
  removed_count: number;
  remaining_subscription_ids: string[];
}

export type FinanceEventKind = 'rss_article' | 'market_candle_closed';
export type FinanceDelivery = 'immediate' | 'silent';

export interface FinanceSubscriptionSource {
  source_name: string;
  catalog_source_id: string | null;
  catalog_name: string | null;
  provider: string | null;
  feed_id: string | null;
  feed_type: FeedType | null;
  enabled: boolean | null;
  status: DataFeedConfig['status'] | null;
}

export interface FinanceSubscription {
  subscription_id: string;
  session_key: string;
  event_kinds: FinanceEventKind[];
  source_names: string[];
  matches_all_sources: boolean;
  sources: FinanceSubscriptionSource[];
  category_tags: string[];
  watch_terms: string[];
  venues: string[];
  symbols: string[];
  timeframes: string[];
  delivery: FinanceDelivery;
  cooldown_secs: number;
  max_immediate_per_hour: number;
}

export interface FinanceSubscriptionsResponse {
  subscriptions: FinanceSubscription[];
  count: number;
}

export interface CreateFinanceSubscriptionRequest {
  session_key: string;
  event_kinds?: FinanceEventKind[];
  catalog_source_ids?: string[];
  source_names?: string[];
  match_all_sources?: boolean;
  category_tags?: string[];
  watch_terms?: string[];
  venues?: string[];
  symbols?: string[];
  timeframes?: string[];
  delivery?: FinanceDelivery;
  cooldown_secs?: number;
  max_immediate_per_hour?: number;
}

export interface CreateFinanceSubscriptionResponse {
  subscription: FinanceSubscription;
  created: boolean;
}

export interface DeleteFinanceSubscriptionResponse {
  subscription_id: string;
  removed: boolean;
}

// ---------------------------------------------------------------------------
// API client
// ---------------------------------------------------------------------------

export const dataFeedsApi = {
  list: () => api.get<DataFeedConfig[]>('/api/v1/data-feeds'),

  catalog: () => api.get<FeedCatalogEntry[]>('/api/v1/data-feeds/catalog'),

  summaries: () => api.get<FeedSummary[]>('/api/v1/data-feeds/summary'),

  candleStreams: (params?: {
    source_name?: string;
    venue?: string;
    symbol?: string;
    timeframe?: string;
    limit?: number;
  }) => {
    const query = new URLSearchParams();
    if (params?.source_name) query.set('source_name', params.source_name);
    if (params?.venue) query.set('venue', params.venue);
    if (params?.symbol) query.set('symbol', params.symbol);
    if (params?.timeframe) query.set('timeframe', params.timeframe);
    if (params?.limit) query.set('limit', String(params.limit));
    const qs = query.toString();
    return api.get<CandleStreamsResponse>(
      `/api/v1/data-feeds/market-data/candle-streams${qs ? `?${qs}` : ''}`,
    );
  },

  latestCandle: (params: {
    source_name?: string;
    venue: string;
    symbol: string;
    timeframe: string;
  }) => {
    const query = new URLSearchParams();
    if (params.source_name) query.set('source_name', params.source_name);
    query.set('venue', params.venue);
    query.set('symbol', params.symbol);
    query.set('timeframe', params.timeframe);
    return api.get<LatestMarketCandleResponse>(
      `/api/v1/data-feeds/market-data/candles/latest?${query.toString()}`,
    );
  },

  recentCandles: (params: {
    source_name?: string;
    venue: string;
    symbol: string;
    timeframe: string;
    limit?: number;
  }) => {
    const query = new URLSearchParams();
    if (params.source_name) query.set('source_name', params.source_name);
    query.set('venue', params.venue);
    query.set('symbol', params.symbol);
    query.set('timeframe', params.timeframe);
    if (params.limit) query.set('limit', String(params.limit));
    return api.get<RecentMarketCandlesResponse>(
      `/api/v1/data-feeds/market-data/candles/recent?${query.toString()}`,
    );
  },

  candles: (params: {
    source_name?: string;
    venue: string;
    symbol: string;
    timeframe: string;
    start: string;
    end: string;
    limit?: number;
  }) => {
    const query = new URLSearchParams();
    if (params.source_name) query.set('source_name', params.source_name);
    query.set('venue', params.venue);
    query.set('symbol', params.symbol);
    query.set('timeframe', params.timeframe);
    query.set('start', params.start);
    query.set('end', params.end);
    if (params.limit) query.set('limit', String(params.limit));
    return api.get<MarketCandlesResponse>(
      `/api/v1/data-feeds/market-data/candles?${query.toString()}`,
    );
  },

  candleFreshness: (params: {
    source_name?: string;
    venue: string;
    symbol: string;
    timeframe: string;
    as_of?: string;
    stale_after_secs?: number;
  }) => {
    const query = new URLSearchParams();
    if (params.source_name) query.set('source_name', params.source_name);
    query.set('venue', params.venue);
    query.set('symbol', params.symbol);
    query.set('timeframe', params.timeframe);
    if (params.as_of) query.set('as_of', params.as_of);
    if (params.stale_after_secs) query.set('stale_after_secs', String(params.stale_after_secs));
    return api.get<MarketCandleFreshnessResponse>(
      `/api/v1/data-feeds/market-data/candles/freshness?${query.toString()}`,
    );
  },

  candleGaps: (params: {
    source_name?: string;
    venue: string;
    symbol: string;
    timeframe: string;
    start: string;
    end: string;
  }) => {
    const query = new URLSearchParams();
    if (params.source_name) query.set('source_name', params.source_name);
    query.set('venue', params.venue);
    query.set('symbol', params.symbol);
    query.set('timeframe', params.timeframe);
    query.set('start', params.start);
    query.set('end', params.end);
    return api.get<MarketCandleGapsResponse>(
      `/api/v1/data-feeds/market-data/candles/gaps?${query.toString()}`,
    );
  },

  get: (id: string) => api.get<DataFeedConfig>(`/api/v1/data-feeds/${id}`),

  create: (feed: CreateFeedRequest) => api.post<DataFeedConfig>('/api/v1/data-feeds', feed),

  update: (id: string, feed: Partial<CreateFeedRequest>) =>
    api.put<DataFeedConfig>(`/api/v1/data-feeds/${id}`, feed),

  delete: (id: string) => api.del(`/api/v1/data-feeds/${id}`),

  toggle: (id: string) => api.put<DataFeedConfig>(`/api/v1/data-feeds/${id}/toggle`),

  enableCatalogEntry: (id: string, body?: EnableCatalogEntryRequest) =>
    api.post<DataFeedConfig>(`/api/v1/data-feeds/catalog/${id}/enable`, body),

  disableCatalogEntry: (id: string) =>
    api.post<DataFeedConfig>(`/api/v1/data-feeds/catalog/${id}/disable`),

  unsubscribeCatalogEntry: (id: string, body?: UnsubscribeCatalogEntryRequest) =>
    api.post<UnsubscribeCatalogEntryResponse>(`/api/v1/data-feeds/catalog/${id}/unsubscribe`, body),

  financeSubscriptions: () =>
    api.get<FinanceSubscriptionsResponse>('/api/v1/data-feeds/finance/subscriptions'),

  financeBundles: () => api.get<FinanceFeedBundlesResponse>('/api/v1/data-feeds/finance/bundles'),

  createFinanceSubscription: (body: CreateFinanceSubscriptionRequest) =>
    api.post<CreateFinanceSubscriptionResponse>('/api/v1/data-feeds/finance/subscriptions', body),

  getFinanceSubscription: (id: string) =>
    api.get<FinanceSubscription>(`/api/v1/data-feeds/finance/subscriptions/${id}`),

  deleteFinanceSubscription: (id: string) =>
    api.del<DeleteFinanceSubscriptionResponse>(`/api/v1/data-feeds/finance/subscriptions/${id}`),

  events: (
    id: string,
    params?: {
      since?: string;
      event_kinds?: FinanceEventKind[];
      limit?: number;
      offset?: number;
    },
  ) => {
    const query = new URLSearchParams();
    if (params?.since) query.set('since', params.since);
    if (params?.event_kinds?.length) query.set('event_kinds', params.event_kinds.join(','));
    if (params?.limit) query.set('limit', String(params.limit));
    if (params?.offset) query.set('offset', String(params.offset));
    const qs = query.toString();
    return api.get<FeedEventsResponse>(`/api/v1/data-feeds/${id}/events${qs ? `?${qs}` : ''}`);
  },
};
