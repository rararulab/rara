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

/** Paths that should never carry an Authorization header. */
const AUTH_EXCLUDED_PATHS = ['/api/v1/auth/login', '/api/v1/auth/register', '/api/v1/auth/refresh'];

let isRefreshing = false;
let refreshPromise: Promise<boolean> | null = null;

async function tryRefreshToken(): Promise<boolean> {
  if (isRefreshing && refreshPromise) return refreshPromise;
  isRefreshing = true;
  refreshPromise = (async () => {
    const refreshToken = localStorage.getItem('refresh_token');
    if (!refreshToken) return false;
    try {
      const res = await fetch(`${BASE_URL}/api/v1/auth/refresh`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ refresh_token: refreshToken }),
      });
      if (!res.ok) return false;
      const data = await res.json();
      localStorage.setItem('access_token', data.access_token);
      localStorage.setItem('refresh_token', data.refresh_token);
      return true;
    } catch {
      return false;
    } finally {
      isRefreshing = false;
      refreshPromise = null;
    }
  })();
  return refreshPromise;
}

function clearAuthAndRedirect() {
  localStorage.removeItem('access_token');
  localStorage.removeItem('refresh_token');
  localStorage.removeItem('user');
  if (window.location.pathname !== '/login') {
    window.location.href = '/login';
  }
}

async function request<T>(path: string, options?: RequestInit & { timeoutMs?: number; _retry?: boolean }): Promise<T> {
  const { timeoutMs = DEFAULT_TIMEOUT_MS, _retry, ...fetchOptions } = options ?? {};
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);

  const token = localStorage.getItem('access_token');
  const headers: Record<string, string> = {
    'Content-Type': 'application/json',
    ...(fetchOptions?.headers as Record<string, string>),
  };
  if (token && !AUTH_EXCLUDED_PATHS.includes(path)) {
    headers['Authorization'] = `Bearer ${token}`;
  }

  try {
    const res = await fetch(`${BASE_URL}${path}`, {
      ...fetchOptions,
      headers,
      signal: controller.signal,
    });

    // Handle 401: try refresh once, then retry the original request
    if (res.status === 401 && !_retry && !AUTH_EXCLUDED_PATHS.includes(path)) {
      const refreshed = await tryRefreshToken();
      if (refreshed) {
        return request<T>(path, { ...options, _retry: true });
      }
      clearAuthAndRedirect();
      throw new ApiError(401, 'Session expired');
    }

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

import type {
  SettingsMap, SettingValue, SettingsPatch,
  LoginRequest, RegisterRequest, RefreshRequest,
  AuthResponse, UserProfile, UserInfo, PlatformInfo,
  InviteCode, ChangePasswordRequest, LinkCodeResponse,
} from './types';

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
};

export const authApi = {
  login: (data: LoginRequest) =>
    request<AuthResponse>('/api/v1/auth/login', { method: 'POST', body: JSON.stringify(data) }),
  register: (data: RegisterRequest) =>
    request<AuthResponse>('/api/v1/auth/register', { method: 'POST', body: JSON.stringify(data) }),
  refresh: (data: RefreshRequest) =>
    request<AuthResponse>('/api/v1/auth/refresh', { method: 'POST', body: JSON.stringify(data) }),
  me: () => request<UserProfile>('/api/v1/users/me'),
  changePassword: (data: ChangePasswordRequest) =>
    request<void>('/api/v1/users/me/password', { method: 'PUT', body: JSON.stringify(data) }),
  generateLinkCode: (direction: string) =>
    request<LinkCodeResponse>('/api/v1/users/me/link-code', { method: 'POST', body: JSON.stringify({ direction }) }),
};

export const adminApi = {
  listUsers: () => request<(UserInfo & { platforms?: PlatformInfo[] })[]>('/api/v1/admin/users'),
  disableUser: (id: string) => request<void>(`/api/v1/admin/users/${id}`, { method: 'DELETE' }),
  createInviteCode: () => request<InviteCode>('/api/v1/admin/invite-codes', { method: 'POST' }),
  listInviteCodes: () => request<InviteCode[]>('/api/v1/admin/invite-codes'),
};

export const settingsApi = {
  list: () => request<SettingsMap>('/api/v1/settings'),
  get: (key: string) => request<SettingValue>(`/api/v1/settings/${key}`),
  set: (key: string, value: string) =>
    request<void>(`/api/v1/settings/${key}`, {
      method: 'PUT',
      body: JSON.stringify({ value }),
    }),
  delete: (key: string) =>
    request<void>(`/api/v1/settings/${key}`, { method: 'DELETE' }),
  batchUpdate: (patches: SettingsPatch) =>
    request<void>('/api/v1/settings', {
      method: 'PATCH',
      body: JSON.stringify(patches),
    }),
};
