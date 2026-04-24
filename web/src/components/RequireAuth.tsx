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

import type { ReactNode } from 'react';
import { Navigate, useLocation } from 'react-router';

import { clearAuth, getStoredAuth } from '@/api/client';

interface RequireAuthProps {
  children: ReactNode;
}

/**
 * Route guard that redirects to `/login` when the user is not authenticated.
 *
 * Checks `getStoredAuth()` on every render. If the access token or cached
 * principal is missing (or malformed, or has an empty `user_id`), the stored
 * auth is cleared and the user is redirected to `/login?redirect=<pathname>`,
 * preserving the requested path + search string so Login can send them back
 * after sign-in.
 *
 * The `/login` route itself must not be wrapped — it is the fallback
 * destination and wrapping it would cause a redirect loop.
 *
 * This guard complements (does not replace) the 401-driven redirect in the
 * fetch helper and WebSocket builder: it covers pages that render without
 * making any network call on mount.
 */
export function RequireAuth({ children }: RequireAuthProps) {
  const location = useLocation();

  if (getStoredAuth() === null) {
    // Evict any partial/stale state so the fetch helper agrees with us.
    clearAuth();
    const redirect = encodeURIComponent(location.pathname + location.search);
    return <Navigate to={`/login?redirect=${redirect}`} replace />;
  }

  return <>{children}</>;
}
