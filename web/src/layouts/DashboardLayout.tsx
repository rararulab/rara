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
  Globe,
  Code,
  Database,
  ExternalLink,
  PanelLeftClose,
  PanelLeftOpen,
  Sun,
  Moon,
  Monitor,
} from 'lucide-react';
import { cn } from '@/lib/utils';
import { Separator } from '@/components/ui/separator';
import { Button } from '@/components/ui/button';
import { useLocalStorage } from '@/hooks/use-local-storage';
import { useTheme } from '@/hooks/use-theme';
import { useServerStatus } from '@/hooks/use-server-status';

const THEME_META = {
  system: { icon: Monitor, label: 'System' },
  light:  { icon: Sun,     label: 'Light' },
  dark:   { icon: Moon,    label: 'Dark' },
} as const;

const navItems = [
  { to: '/agent', icon: Bot, label: 'Agent' },
  { to: '/jobs', icon: Briefcase, label: 'Jobs' },
  { to: '/settings', icon: SettingsIcon, label: 'Settings' },
];

function ServerStatus({ collapsed }: { collapsed: boolean }) {
  const { isOnline, isChecking } = useServerStatus();

  return (
    <div className={cn('flex items-center gap-2 py-3 text-xs text-muted-foreground', collapsed ? 'justify-center px-2' : 'px-4')}>
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

function ThemeToggle({ collapsed }: { collapsed: boolean }) {
  const { theme, cycleTheme } = useTheme();
  const meta = THEME_META[theme];
  const Icon = meta.icon;

  return (
    <Button
      variant="ghost"
      size={collapsed ? 'icon' : 'sm'}
      className={cn('shrink-0', collapsed ? 'mx-auto h-8 w-8' : 'mx-4 justify-start gap-2')}
      onClick={cycleTheme}
      title={`Theme: ${meta.label}`}
    >
      <Icon className="h-4 w-4 shrink-0" />
      {!collapsed && <span className="text-xs text-muted-foreground">{meta.label}</span>}
    </Button>
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

/** Routes that need full bleed when they match as a prefix (e.g. /typst/:id). */
const FULL_BLEED_PREFIXES = ['/jobs/typst/'];

export default function DashboardLayout() {
  const [collapsed, setCollapsed] = useLocalStorage('sidebar-collapsed', false);
  const isWide = useIsWide();
  const location = useLocation();
  const isFullBleed = FULL_BLEED_ROUTES.has(location.pathname) || FULL_BLEED_PREFIXES.some(p => location.pathname.startsWith(p));

  useEffect(() => {
    setCollapsed(!isWide);
  }, [isWide, setCollapsed]);

  return (
    <div className="flex h-screen">
      {/* Sidebar */}
      <aside className={cn('border-r bg-card flex flex-col transition-all duration-200', collapsed ? 'w-16' : 'w-64')}>
        <div className={cn('flex items-center', collapsed ? 'justify-center p-4' : 'justify-between p-6')}>
          {!collapsed && <h1 className="text-xl font-bold">Rara</h1>}
          <Button
            variant="ghost"
            size="icon"
            className="h-8 w-8 shrink-0"
            onClick={() => setCollapsed((prev) => !prev)}
          >
            {collapsed ? <PanelLeftOpen className="h-4 w-4" /> : <PanelLeftClose className="h-4 w-4" />}
          </Button>
        </div>
        <Separator />
        <nav className={cn('flex-1 space-y-1', collapsed ? 'p-2' : 'p-4')}>
          {navItems.map((item) => (
            <NavLink
              key={item.to}
              to={item.to}
              title={collapsed ? item.label : undefined}
              className={({ isActive }) => {
                const active = isActive || location.pathname.startsWith(item.to + '/');
                return cn(
                  'flex items-center rounded-md text-sm font-medium transition-colors',
                  collapsed ? 'justify-center px-2 py-2' : 'gap-3 px-3 py-2',
                  active
                    ? 'bg-accent text-accent-foreground'
                    : 'text-muted-foreground hover:bg-accent hover:text-accent-foreground'
                );
              }}
            >
              <item.icon className="h-4 w-4 shrink-0" />
              {!collapsed && item.label}
            </NavLink>
          ))}
          <a
            href="/swagger-ui"
            target="_blank"
            rel="noopener noreferrer"
            title={collapsed ? 'API Docs' : undefined}
            className={cn(
              'flex items-center rounded-md text-sm font-medium transition-colors text-muted-foreground hover:bg-accent hover:text-accent-foreground',
              collapsed ? 'justify-center px-2 py-2' : 'gap-3 px-3 py-2'
            )}
          >
            <Code className="h-4 w-4 shrink-0" />
            {!collapsed && (
              <>
                <span>API Docs</span>
                <ExternalLink className="h-3.5 w-3.5 ml-auto shrink-0 opacity-70" />
              </>
            )}
          </a>
          <a
            href="http://localhost:9001"
            target="_blank"
            rel="noopener noreferrer"
            title={collapsed ? 'Object Storage' : undefined}
            className={cn(
              'flex items-center rounded-md text-sm font-medium transition-colors text-muted-foreground hover:bg-accent hover:text-accent-foreground',
              collapsed ? 'justify-center px-2 py-2' : 'gap-3 px-3 py-2'
            )}
          >
            <Database className="h-4 w-4 shrink-0" />
            {!collapsed && (
              <>
                <span>Object Storage</span>
                <ExternalLink className="h-3.5 w-3.5 ml-auto shrink-0 opacity-70" />
              </>
            )}
          </a>
          <a
            href="http://localhost:11235/dashboard"
            target="_blank"
            rel="noopener noreferrer"
            title={collapsed ? 'Crawl4AI UI' : undefined}
            className={cn(
              'flex items-center rounded-md text-sm font-medium transition-colors text-muted-foreground hover:bg-accent hover:text-accent-foreground',
              collapsed ? 'justify-center px-2 py-2' : 'gap-3 px-3 py-2'
            )}
          >
            <Globe className="h-4 w-4 shrink-0" />
            {!collapsed && (
              <>
                <span>Crawl4AI UI</span>
                <ExternalLink className="h-3.5 w-3.5 ml-auto shrink-0 opacity-70" />
              </>
            )}
          </a>
        </nav>
        <Separator />
        <div className="py-2">
          <ThemeToggle collapsed={collapsed} />
        </div>
        <Separator />
        <ServerStatus collapsed={collapsed} />
      </aside>

      {/* Main content */}
      <main className={cn('flex-1 flex flex-col', isFullBleed ? 'overflow-hidden' : 'overflow-auto')}>
        <OfflineBanner />
        <div className={isFullBleed ? 'flex-1 min-h-0' : 'p-8'}>
          <Outlet />
        </div>
      </main>
    </div>
  );
}
