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

import { Outlet, useLocation } from 'react-router';
import { cn } from '@/lib/utils';
import { useServerStatus } from '@/hooks/use-server-status';

function OfflineBanner() {
  const { isOnline, isChecking } = useServerStatus();
  if (isOnline || isChecking) return null;
  return (
    <div className="bg-destructive/10 border-b border-destructive/20 px-4 py-2 text-center text-sm text-destructive">
      Server is currently offline. Requests are paused and will resume automatically when the server is back.
    </div>
  );
}

/** Routes that need zero padding in the main content area. */
const FULL_BLEED_ROUTES = new Set(['/agent', '/jobs', '/docs']);

/** Routes that need full bleed when they match as a prefix. */
const FULL_BLEED_PREFIXES: string[] = [];

export default function DashboardLayout() {
  const location = useLocation();
  const isFullBleed = FULL_BLEED_ROUTES.has(location.pathname) || FULL_BLEED_PREFIXES.some(p => location.pathname.startsWith(p));

  return (
    <div className="flex h-screen bg-transparent">
      <main className={cn('relative flex min-w-0 flex-1 flex-col', isFullBleed ? 'overflow-hidden' : 'overflow-auto')}>
        <OfflineBanner />
        <div className={isFullBleed ? 'flex-1 min-h-0 p-2 md:p-3' : 'p-4 md:p-6'}>
          <Outlet />
        </div>
      </main>
    </div>
  );
}
