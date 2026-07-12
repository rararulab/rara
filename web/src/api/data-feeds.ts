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
  last_event_at: string | null;
  lag_seconds: number | null;
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

  createFinanceSubscription: (body: CreateFinanceSubscriptionRequest) =>
    api.post<CreateFinanceSubscriptionResponse>('/api/v1/data-feeds/finance/subscriptions', body),

  getFinanceSubscription: (id: string) =>
    api.get<FinanceSubscription>(`/api/v1/data-feeds/finance/subscriptions/${id}`),

  deleteFinanceSubscription: (id: string) =>
    api.del<DeleteFinanceSubscriptionResponse>(`/api/v1/data-feeds/finance/subscriptions/${id}`),

  events: (id: string, params?: { since?: string; limit?: number; offset?: number }) => {
    const query = new URLSearchParams();
    if (params?.since) query.set('since', params.since);
    if (params?.limit) query.set('limit', String(params.limit));
    if (params?.offset) query.set('offset', String(params.offset));
    const qs = query.toString();
    return api.get<FeedEventsResponse>(`/api/v1/data-feeds/${id}/events${qs ? `?${qs}` : ''}`);
  },
};
