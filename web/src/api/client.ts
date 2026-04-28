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

/** localStorage key holding the owner bearer token entered at /login. */
export const ACCESS_TOKEN_KEY = 'access_token';

/** localStorage key holding the authenticated principal `{ user_id, role, is_admin }`. */
export const AUTH_USER_KEY = 'auth_user';

/** Shape of the authenticated principal cached in localStorage. */
export interface AuthUser {
  user_id: string;
  role: string;
  is_admin: boolean;
}

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

/** Read the access token from localStorage, or `null` if the user is logged out. */
export function getAccessToken(): string | null {
  if (typeof window === 'undefined') return null;
  return localStorage.getItem(ACCESS_TOKEN_KEY);
}

/** Read the cached authenticated principal, or `null` if the user is logged out. */
export function getAuthUser(): AuthUser | null {
  if (typeof window === 'undefined') return null;
  const raw = localStorage.getItem(AUTH_USER_KEY);
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as unknown;
    if (
      parsed &&
      typeof parsed === 'object' &&
      typeof (parsed as AuthUser).user_id === 'string' &&
      typeof (parsed as AuthUser).role === 'string'
    ) {
      return parsed as AuthUser;
    }
  } catch {
    // Malformed entry — treat as logged out.
  }
  return null;
}

/**
 * Read both the access token and authenticated principal from localStorage.
 *
 * Returns `null` if either is missing, if `auth_user` is malformed JSON, or if
 * the cached principal has an empty `user_id`. Shared by the fetch helper and
 * the `<RequireAuth>` route guard so there is a single source of truth for
 * "is the user logged in".
 */
export function getStoredAuth(): { token: string; user: AuthUser } | null {
  const token = getAccessToken();
  if (!token) return null;
  const user = getAuthUser();
  if (!user || user.user_id === '') return null;
  return { token, user };
}

/** Persist the access token and principal after a successful login. */
export function setAuth(token: string, user: AuthUser): void {
  localStorage.setItem(ACCESS_TOKEN_KEY, token);
  localStorage.setItem(AUTH_USER_KEY, JSON.stringify(user));
}

/** Clear auth state (token + principal). */
export function clearAuth(): void {
  localStorage.removeItem(ACCESS_TOKEN_KEY);
  localStorage.removeItem(AUTH_USER_KEY);
}

/**
 * Clear auth state and redirect to `/login?redirect=<current-path>` unless
 * we're already there. Intended for 401 responses from admin endpoints.
 */
export function redirectToLogin(): void {
  clearAuth();
  if (typeof window === 'undefined') return;
  if (window.location.pathname === '/login') return;
  const redirect = encodeURIComponent(window.location.pathname + window.location.search);
  window.location.href = `/login?redirect=${redirect}`;
}

/**
 * Build common request headers, including the `Authorization: Bearer` header
 * when an access token is present.
 */
export function apiHeaders(extra?: Record<string, string>): Record<string, string> {
  const token = getAccessToken();
  const auth: Record<string, string> = token ? { Authorization: `Bearer ${token}` } : {};
  return {
    'Content-Type': 'application/json',
    ...auth,
    ...extra,
  };
}

/**
 * Error thrown for non-2xx HTTP responses from the rara backend.
 *
 * `requestId` carries the `x-request-id` response header (a 32-hex OTel
 * trace_id) when the backend emitted one. It is the join key into Langfuse
 * and Loki — surface it in any user-visible error message so support can
 * resolve a trace from a single paste. See
 * `specs/issue-1975-trace-id-response-header.spec.md`.
 */
export class ApiError extends Error {
  readonly status: number;
  readonly requestId?: string;

  constructor(status: number, message: string, requestId?: string) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
    if (requestId) this.requestId = requestId;
  }
}

/** Lower-cased response header carrying the backend's OTel trace_id. */
const REQUEST_ID_HEADER = 'x-request-id';

/**
 * Construct an `ApiError` and emit one `console.error` line that includes
 * the request id. This is the dominant error surface (the codebase has no
 * single global toast/error renderer); a developer triaging a user bug
 * report can copy the id from devtools straight into Langfuse / Loki.
 */
function raiseApiError(status: number, message: string, requestId?: string): ApiError {
  const err = new ApiError(status, message, requestId);
  if (requestId) {
    console.error(`[api] ${status} ${message} (request_id=${requestId})`);
  } else {
    console.error(`[api] ${status} ${message}`);
  }
  return err;
}

const DEFAULT_TIMEOUT_MS = 60_000;

/**
 * `AbortSignal.any` is part of ES2024 / WHATWG DOM but our `lib` target
 * is ES2022, so the method isn't in the builtin typings yet. Declare a
 * narrow structural type for the optional static method — this keeps
 * the feature-detect branch fully typed without reaching for `as any`.
 */
interface AbortSignalWithAny {
  any?: (signals: AbortSignal[]) => AbortSignal;
}

/** Combine an internal timeout signal with a caller-provided signal so
 *  aborting either cancels the underlying fetch. `AbortSignal.any` is
 *  available in modern browsers; we fall back to a manual relay when the
 *  runtime doesn't expose it. */
function composeSignals(internal: AbortSignal, external?: AbortSignal | null): AbortSignal {
  if (!external) return internal;
  const native: AbortSignalWithAny = AbortSignal;
  if (native.any) return native.any([internal, external]);
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

/**
 * Handle a 401 response from any admin endpoint by clearing auth state and
 * redirecting to the login page. Exported so callers that hand-roll `fetch`
 * (e.g. SSE / streaming endpoints) can funnel through the same policy.
 */
export function handleUnauthorized(): void {
  redirectToLogin();
}

async function request<T>(
  path: string,
  options?: RequestInit & { timeoutMs?: number },
): Promise<T> {
  const { timeoutMs = DEFAULT_TIMEOUT_MS, signal: externalSignal, ...fetchOptions } = options ?? {};
  const timeoutController = new AbortController();
  const timer = setTimeout(() => timeoutController.abort(), timeoutMs);
  const signal = composeSignals(timeoutController.signal, externalSignal);

  const token = getAccessToken();
  const headers: Record<string, string> = {
    'Content-Type': 'application/json',
    ...(token ? { Authorization: `Bearer ${token}` } : {}),
    ...(fetchOptions?.headers as Record<string, string>),
  };

  try {
    const res = await fetch(resolveUrl(path), {
      ...fetchOptions,
      headers,
      signal,
    });

    const requestId = res.headers.get(REQUEST_ID_HEADER) ?? undefined;

    if (res.status === 401) {
      handleUnauthorized();
      throw raiseApiError(401, 'Unauthorized', requestId);
    }

    if (!res.ok) {
      const text = await res.text();
      throw raiseApiError(res.status, text || res.statusText, requestId);
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

  const token = getAccessToken();
  const headers: Record<string, string> = {
    ...(token ? { Authorization: `Bearer ${token}` } : {}),
    ...(fetchOptions?.headers as Record<string, string>),
  };

  try {
    const res = await fetch(resolveUrl(path), {
      ...fetchOptions,
      headers,
      signal: controller.signal,
    });
    const requestId = res.headers.get(REQUEST_ID_HEADER) ?? undefined;
    if (res.status === 401) {
      handleUnauthorized();
      throw raiseApiError(401, 'Unauthorized', requestId);
    }
    if (!res.ok) {
      const text = await res.text();
      throw raiseApiError(res.status, text || res.statusText, requestId);
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
