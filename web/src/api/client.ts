/*
 * Copyright 2025 Crrow
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

const BASE_URL = import.meta.env.VITE_API_URL || '';

class ApiError extends Error {
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
  }
}

const DEFAULT_TIMEOUT_MS = 60_000;

async function request<T>(path: string, options?: RequestInit & { timeoutMs?: number }): Promise<T> {
  const { timeoutMs = DEFAULT_TIMEOUT_MS, ...fetchOptions } = options ?? {};
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);

  try {
    const res = await fetch(`${BASE_URL}${path}`, {
      headers: {
        'Content-Type': 'application/json',
        ...fetchOptions?.headers,
      },
      ...fetchOptions,
      signal: controller.signal,
    });
    if (!res.ok) {
      const text = await res.text();
      throw new ApiError(res.status, text || res.statusText);
    }
    if (res.status === 204) return undefined as T;
    return res.json();
  } catch (err) {
    if (err instanceof DOMException && err.name === 'AbortError') {
      throw new ApiError(0, `Request timeout after ${timeoutMs}ms`);
    }
    throw err;
  } finally {
    clearTimeout(timer);
  }
}

async function requestBlob(path: string, options?: RequestInit & { timeoutMs?: number }): Promise<Blob> {
  const { timeoutMs = DEFAULT_TIMEOUT_MS, ...fetchOptions } = options ?? {};
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);

  try {
    const res = await fetch(`${BASE_URL}${path}`, {
      ...fetchOptions,
      signal: controller.signal,
    });
    if (!res.ok) {
      const text = await res.text();
      throw new ApiError(res.status, text || res.statusText);
    }
    return res.blob();
  } catch (err) {
    if (err instanceof DOMException && err.name === 'AbortError') {
      throw new ApiError(0, `Request timeout after ${timeoutMs}ms`);
    }
    throw err;
  } finally {
    clearTimeout(timer);
  }
}

import type { PipelineDiscoveredJob, PaginatedDiscoveredJobs, DiscoveredJobsStats, LlmfitRecommendationsResponse, OllamaHealthResponse, OllamaModelListResponse, OllamaModelInfo, DispatcherStatus, TaskRecord, AgentTaskKind, TaskStatus } from './types';

export const api = {
  get: <T>(path: string) => request<T>(path),
  post: <T>(path: string, body?: unknown) =>
    request<T>(path, { method: 'POST', body: body ? JSON.stringify(body) : undefined }),
  put: <T>(path: string, body?: unknown) =>
    request<T>(path, { method: 'PUT', body: body ? JSON.stringify(body) : undefined }),
  patch: <T>(path: string, body?: unknown) =>
    request<T>(path, { method: 'PATCH', body: body ? JSON.stringify(body) : undefined }),
  del: <T>(path: string) => request<T>(path, { method: 'DELETE' }),
  blob: (path: string) => requestBlob(path),

  // -- Pipeline --

  fetchPipelineRunJobs(runId: string): Promise<PipelineDiscoveredJob[]> {
    return request<PipelineDiscoveredJob[]>(`/api/v1/pipeline/runs/${runId}/jobs`);
  },

  fetchDiscoveredJobs(params?: {
    action?: string;
    min_score?: number;
    max_score?: number;
    run_id?: string;
    sort_by?: string;
    limit?: number;
    offset?: number;
  }): Promise<PaginatedDiscoveredJobs> {
    const searchParams = new URLSearchParams();
    if (params) {
      if (params.action) searchParams.set('action', params.action);
      if (params.min_score != null) searchParams.set('min_score', String(params.min_score));
      if (params.max_score != null) searchParams.set('max_score', String(params.max_score));
      if (params.run_id) searchParams.set('run_id', params.run_id);
      if (params.sort_by) searchParams.set('sort_by', params.sort_by);
      if (params.limit != null) searchParams.set('limit', String(params.limit));
      if (params.offset != null) searchParams.set('offset', String(params.offset));
    }
    const qs = searchParams.toString();
    return request<PaginatedDiscoveredJobs>(`/api/v1/pipeline/discovered-jobs${qs ? `?${qs}` : ''}`);
  },

  fetchDiscoveredJobsStats(): Promise<DiscoveredJobsStats> {
    return request<DiscoveredJobsStats>('/api/v1/pipeline/discovered-jobs/stats');
  },

  getOllamaModelRecommendations: (limit = 10) =>
    request<LlmfitRecommendationsResponse>(`/api/v1/settings/ollama/model-recommendations?limit=${limit}`),

  ollamaHealth: () => request<OllamaHealthResponse>('/api/v1/settings/ollama/health'),
  ollamaListModels: () => request<OllamaModelListResponse>('/api/v1/settings/ollama/models'),
  ollamaDeleteModel: (name: string) =>
    request<void>('/api/v1/settings/ollama/models', {
      method: 'DELETE',
      body: JSON.stringify({ name }),
    }),
  ollamaModelInfo: (name: string) =>
    request<OllamaModelInfo>(`/api/v1/settings/ollama/models/${encodeURIComponent(name)}/info`),

  updateDiscoveredJobAction(id: string, action: string): Promise<PipelineDiscoveredJob> {
    return request<PipelineDiscoveredJob>(`/api/v1/pipeline/discovered-jobs/${id}`, {
      method: 'PATCH',
      body: JSON.stringify({ action }),
    });
  },

  // -- Agent Dispatcher --

  fetchDispatcherStatus(): Promise<DispatcherStatus> {
    return request<DispatcherStatus>('/api/dispatcher/status');
  },

  fetchDispatcherHistory(params?: {
    limit?: number;
    kind?: AgentTaskKind;
    status?: TaskStatus;
  }): Promise<TaskRecord[]> {
    const searchParams = new URLSearchParams();
    if (params?.limit) searchParams.set('limit', String(params.limit));
    if (params?.kind) searchParams.set('kind', params.kind);
    if (params?.status) searchParams.set('status', params.status);
    const query = searchParams.toString();
    return request<TaskRecord[]>(`/api/dispatcher/history${query ? `?${query}` : ''}`);
  },

  cancelDispatcherTask(taskId: string): Promise<{ success: boolean }> {
    return request<{ success: boolean }>(`/api/dispatcher/cancel/${taskId}`, { method: 'POST' });
  },
};
