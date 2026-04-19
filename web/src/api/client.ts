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

const STORAGE_KEY = 'rara_backend_url';

/** Derive a sensible default backend URL from the current page hostname. */
function defaultBackendUrl(): string {
  const host = typeof window !== 'undefined' ? window.location.hostname : 'localhost';
  return `http://${host}:25555`;
}

/** Read the backend URL from localStorage, env, or fallback to default. */
export function getBackendUrl(): string {
  if (typeof window !== 'undefined') {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored) return stored;
  }
  return import.meta.env.VITE_API_URL || defaultBackendUrl();
}

/** Persist backend URL and reload the page so all clients pick it up. */
export function setBackendUrl(url: string) {
  localStorage.setItem(STORAGE_KEY, url);
  window.location.reload();
}

/** True when the user has explicitly set a custom backend URL. */
function hasCustomBackendUrl(): boolean {
  return typeof window !== 'undefined' && localStorage.getItem(STORAGE_KEY) !== null;
}

/**
 * Resolve the fetch URL for a given API path.
 *
 * When no custom URL is stored we use relative paths so the Vite dev proxy
 * can forward `/api/...` requests.  When a custom URL is set we bypass the
 * proxy and hit the backend directly.
 */
export function resolveUrl(path: string): string {
  if (hasCustomBackendUrl()) {
    return `${getBackendUrl()}${path}`;
  }
  return path;
}

export const BASE_URL = '';

/** Build common request headers. */
export function apiHeaders(extra?: Record<string, string>): Record<string, string> {
  return {
    'Content-Type': 'application/json',
    ...extra,
  };
}

class ApiError extends Error {
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
  }
}

const DEFAULT_TIMEOUT_MS = 60_000;

/** Combine an internal timeout signal with a caller-provided signal so
 *  aborting either cancels the underlying fetch. `AbortSignal.any` is
 *  available in modern browsers; we fall back to a manual relay when the
 *  runtime doesn't expose it. */
function composeSignals(internal: AbortSignal, external?: AbortSignal | null): AbortSignal {
  if (!external) return internal;
  const any = (AbortSignal as any).any as ((signals: AbortSignal[]) => AbortSignal) | undefined;
  if (any) return any([internal, external]);
  const relay = new AbortController();
  const onAbort = () => relay.abort();
  if (internal.aborted || external.aborted) {
    relay.abort();
  } else {
    internal.addEventListener('abort', onAbort, { once: true });
    external.addEventListener('abort', onAbort, { once: true });
  }
  return relay.signal;
}

async function request<T>(
  path: string,
  options?: RequestInit & { timeoutMs?: number },
): Promise<T> {
  const { timeoutMs = DEFAULT_TIMEOUT_MS, signal: externalSignal, ...fetchOptions } = options ?? {};
  const timeoutController = new AbortController();
  const timer = setTimeout(() => timeoutController.abort(), timeoutMs);
  const signal = composeSignals(timeoutController.signal, externalSignal);

  const headers: Record<string, string> = {
    'Content-Type': 'application/json',
    ...(fetchOptions?.headers as Record<string, string>),
  };

  try {
    const res = await fetch(resolveUrl(path), {
      ...fetchOptions,
      headers,
      signal,
    });

    if (!res.ok) {
      const text = await res.text();
      throw new ApiError(res.status, text || res.statusText);
    }
    if (res.status === 204) return undefined as T;
    return res.json();
  } catch (err) {
    if (err instanceof DOMException && err.name === 'AbortError') {
      // Distinguish caller cancellation from the internal timeout so
      // callers can decide whether to log or swallow.
      if (externalSignal?.aborted) throw err;
      throw new ApiError(0, `Request timeout after ${timeoutMs}ms`);
    }
    throw err;
  } finally {
    clearTimeout(timer);
  }
}

async function requestBlob(
  path: string,
  options?: RequestInit & { timeoutMs?: number },
): Promise<Blob> {
  const { timeoutMs = DEFAULT_TIMEOUT_MS, ...fetchOptions } = options ?? {};
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);

  try {
    const res = await fetch(resolveUrl(path), {
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

import type { SettingsMap, SettingValue, SettingsPatch } from './types';

/** Callers can pass an `AbortSignal` to cancel in-flight requests
 *  (e.g. a React effect cleanup or a user-triggered dialog close). */
type ApiOptions = { signal?: AbortSignal };

export const api = {
  get: <T>(path: string, options?: ApiOptions) =>
    request<T>(path, options?.signal ? { signal: options.signal } : undefined),
  post: <T>(path: string, body?: unknown, options?: ApiOptions) =>
    request<T>(path, {
      method: 'POST',
      ...(body ? { body: JSON.stringify(body) } : {}),
      ...(options?.signal ? { signal: options.signal } : {}),
    }),
  put: <T>(path: string, body?: unknown, options?: ApiOptions) =>
    request<T>(path, {
      method: 'PUT',
      ...(body ? { body: JSON.stringify(body) } : {}),
      ...(options?.signal ? { signal: options.signal } : {}),
    }),
  patch: <T>(path: string, body?: unknown, options?: ApiOptions) =>
    request<T>(path, {
      method: 'PATCH',
      ...(body ? { body: JSON.stringify(body) } : {}),
      ...(options?.signal ? { signal: options.signal } : {}),
    }),
  del: <T>(path: string, options?: ApiOptions) =>
    request<T>(path, {
      method: 'DELETE',
      ...(options?.signal ? { signal: options.signal } : {}),
    }),
  blob: (path: string) => requestBlob(path),
};

export const settingsApi = {
  list: () => request<SettingsMap>('/api/v1/settings'),
  get: (key: string) => request<SettingValue>(`/api/v1/settings/${key}`),
  set: (key: string, value: string) =>
    request<void>(`/api/v1/settings/${key}`, {
      method: 'PUT',
      body: JSON.stringify({ value }),
    }),
  delete: (key: string) => request<void>(`/api/v1/settings/${key}`, { method: 'DELETE' }),
  batchUpdate: (patches: SettingsPatch) =>
    request<void>('/api/v1/settings', {
      method: 'PATCH',
      body: JSON.stringify(patches),
    }),
};
