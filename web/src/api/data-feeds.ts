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
  feed_type: 'webhook' | 'websocket' | 'polling';
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

export interface CreateFeedRequest {
  name: string;
  feed_type: 'webhook' | 'websocket' | 'polling';
  tags: string[];
  transport: Record<string, unknown>;
  auth: AuthConfig | null;
}

// ---------------------------------------------------------------------------
// API client
// ---------------------------------------------------------------------------

export const dataFeedsApi = {
  list: () => api.get<DataFeedConfig[]>('/api/v1/data-feeds'),

  get: (id: string) => api.get<DataFeedConfig>(`/api/v1/data-feeds/${id}`),

  create: (feed: CreateFeedRequest) => api.post<DataFeedConfig>('/api/v1/data-feeds', feed),

  update: (id: string, feed: Partial<CreateFeedRequest>) =>
    api.put<DataFeedConfig>(`/api/v1/data-feeds/${id}`, feed),

  delete: (id: string) => api.del(`/api/v1/data-feeds/${id}`),

  toggle: (id: string) => api.put<DataFeedConfig>(`/api/v1/data-feeds/${id}/toggle`),

  events: (id: string, params?: { since?: string; limit?: number; offset?: number }) => {
    const query = new URLSearchParams();
    if (params?.since) query.set('since', params.since);
    if (params?.limit) query.set('limit', String(params.limit));
    if (params?.offset) query.set('offset', String(params.offset));
    const qs = query.toString();
    return api.get<FeedEventsResponse>(`/api/v1/data-feeds/${id}/events${qs ? `?${qs}` : ''}`);
  },
};
