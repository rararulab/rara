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

import { useEffect, useSyncExternalStore } from 'react';
import { NavLink, Outlet, useLocation } from 'react-router';
import {
  Bot,
  Briefcase,
  Settings as SettingsIcon,
  PanelLeftClose,
  PanelLeftOpen,
} from 'lucide-react';
import { cn } from '@/lib/utils';
import { Separator } from '@/components/ui/separator';
import { Button } from '@/components/ui/button';
import { useLocalStorage } from '@/hooks/use-local-storage';
import { useServerStatus } from '@/hooks/use-server-status';

const navItems = [
  { to: '/agent', icon: Bot, label: 'Agent' },
  { to: '/jobs', icon: Briefcase, label: 'Jobs' },
  { to: '/settings', icon: SettingsIcon, label: 'Settings' },
];

function ServerStatus({ collapsed }: { collapsed: boolean }) {
  const { isOnline, isChecking } = useServerStatus();

  return (
    <div className={cn('flex items-center gap-2 rounded-xl py-3 text-xs text-muted-foreground', collapsed ? 'justify-center px-2' : 'mx-2 px-3')}>
      <span
        className={cn(
          'h-2 w-2 shrink-0 rounded-full',
          isChecking && 'bg-yellow-400 animate-pulse',
          isOnline && 'bg-green-500',
          !isOnline && !isChecking && 'bg-red-500'
        )}
      />
      {!collapsed && (
        <span>{isChecking ? 'Connecting...' : isOnline ? 'Server online' : 'Server offline'}</span>
      )}
    </div>
  );
}

function OfflineBanner() {
  const { isOnline, isChecking } = useServerStatus();
  if (isOnline || isChecking) return null;
  return (
    <div className="bg-destructive/10 border-b border-destructive/20 px-4 py-2 text-center text-sm text-destructive">
      Server is currently offline. Requests are paused and will resume automatically when the server is back.
    </div>
  );
}

const WIDE_QUERY = '(min-width: 768px)';

function useIsWide(): boolean {
  return useSyncExternalStore(
    (cb) => {
      const mql = window.matchMedia(WIDE_QUERY);
      mql.addEventListener('change', cb);
      return () => mql.removeEventListener('change', cb);
    },
    () => window.matchMedia(WIDE_QUERY).matches,
  );
}

/** Routes that need zero padding in the main content area. */
const FULL_BLEED_ROUTES = new Set(['/agent', '/jobs']);

/** Routes that need full bleed when they match as a prefix. */
const FULL_BLEED_PREFIXES: string[] = [];

export default function DashboardLayout() {
  const [collapsed, setCollapsed] = useLocalStorage('sidebar-collapsed', false);
  const isWide = useIsWide();
  const location = useLocation();
  const isFullBleed = FULL_BLEED_ROUTES.has(location.pathname) || FULL_BLEED_PREFIXES.some(p => location.pathname.startsWith(p));

  useEffect(() => {
    setCollapsed(!isWide);
  }, [isWide, setCollapsed]);

  return (
    <div className="flex h-screen bg-transparent">
      {/* Sidebar */}
      <aside
        className={cn(
          'app-surface relative m-2 flex flex-col overflow-hidden rounded-2xl border transition-all duration-200',
          collapsed ? 'w-16' : 'w-[17rem]'
        )}
      >
        <div className="pointer-events-none absolute inset-x-0 top-0 h-20 bg-gradient-to-b from-primary/12 to-transparent" />
        <div className={cn('relative flex items-center', collapsed ? 'justify-center p-3' : 'justify-between p-5')}>
          {!collapsed && (
            <div className="min-w-0">
              <div className="flex items-center gap-2">
                <span className="inline-flex h-8 w-8 items-center justify-center rounded-xl bg-primary/12 text-primary shadow-sm ring-1 ring-primary/15">
                  <Bot className="h-4 w-4" />
                </span>
                <div>
                  <h1 className="text-base font-semibold tracking-tight">Rara</h1>
                  <p className="text-[11px] text-muted-foreground">Job Copilot Workspace</p>
                </div>
              </div>
            </div>
          )}
          <Button
            variant="ghost"
            size="icon"
            className="h-8 w-8 shrink-0 rounded-lg border border-transparent hover:border-border/80 hover:bg-background/80"
            onClick={() => setCollapsed((prev) => !prev)}
          >
            {collapsed ? <PanelLeftOpen className="h-4 w-4" /> : <PanelLeftClose className="h-4 w-4" />}
          </Button>
        </div>
        <Separator />
        <nav className={cn('flex-1 space-y-1.5', collapsed ? 'p-2' : 'p-3')}>
          {navItems.map((item) => (
            <NavLink
              key={item.to}
              to={item.to}
              title={collapsed ? item.label : undefined}
              className={({ isActive }) => {
                const active = isActive || location.pathname.startsWith(item.to + '/');
                return cn(
                  'group flex items-center rounded-xl text-sm font-medium transition-all',
                  collapsed ? 'justify-center px-2 py-2.5' : 'gap-3 px-3 py-2.5',
                  active
                    ? 'bg-primary/10 text-foreground shadow-sm ring-1 ring-primary/15'
                    : 'text-muted-foreground hover:bg-background/70 hover:text-foreground hover:ring-1 hover:ring-border/70'
                );
              }}
            >
              <item.icon className={cn('h-4 w-4 shrink-0', location.pathname.startsWith(item.to) && 'text-primary')} />
              {!collapsed && <span className="truncate">{item.label}</span>}
            </NavLink>
          ))}
        </nav>
        <Separator />
        <div className="px-1 pb-1">
          <ServerStatus collapsed={collapsed} />
        </div>
      </aside>

      {/* Main content */}
      <main className={cn('flex min-w-0 flex-1 flex-col', isFullBleed ? 'overflow-hidden' : 'overflow-auto')}>
        <OfflineBanner />
        <div className={isFullBleed ? 'flex-1 min-h-0 p-2 md:p-3' : 'p-4 md:p-6'}>
          <Outlet />
        </div>
      </main>
    </div>
  );
}
