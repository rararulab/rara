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

import { BookOpen, MessageSquare, PanelLeft, PanelLeftClose, Settings } from 'lucide-react';
import { useEffect, useState } from 'react';
import { NavLink } from 'react-router';

import { useSettingsModal } from '@/components/settings/SettingsModalProvider';
import { Button } from '@/components/ui/button';
import { useServerStatus } from '@/hooks/use-server-status';
import { cn } from '@/lib/utils';

/** localStorage key for the rail's collapsed preference; distinct from `rara.topology.sidebarCollapsed` (per-page sessions sidebar). */
const RAIL_COLLAPSED_STORAGE_KEY = 'rara.shell.navRailCollapsed';

const RAIL_WIDTH_EXPANDED = 208;
const RAIL_WIDTH_COLLAPSED = 56;

interface NavItem {
  to: string;
  label: string;
  icon: React.ComponentType<{ className?: string }>;
  /** Match nested paths (e.g. `/chat/:rootSessionKey`) under the same item. */
  matchPrefix?: boolean;
}

const NAV_ITEMS: readonly NavItem[] = [
  { to: '/chat', label: 'Chat', icon: MessageSquare, matchPrefix: true },
  { to: '/docs', label: 'Docs', icon: BookOpen },
];

function readCollapsed(): boolean {
  try {
    return window.localStorage.getItem(RAIL_COLLAPSED_STORAGE_KEY) === 'true';
  } catch {
    return false;
  }
}

/**
 * Persistent global nav rail rendered by `DashboardLayout`.
 *
 * Layout — top: brand. Middle: route entries. Bottom: Settings + a
 * single connection-status dot. The rail is collapsible; collapsed
 * state shows icons only and persists in localStorage under
 * `rara.shell.navRailCollapsed`.
 */
export default function NavRail() {
  const { openSettings } = useSettingsModal();
  const [collapsed, setCollapsed] = useState<boolean>(readCollapsed);

  useEffect(() => {
    try {
      window.localStorage.setItem(RAIL_COLLAPSED_STORAGE_KEY, collapsed ? 'true' : 'false');
    } catch {
      // Storage may be unavailable (private browsing); the toggle still
      // works in-memory for the rest of the session.
    }
  }, [collapsed]);

  return (
    <aside
      // Width transitions only; main column has min-w-0 so it absorbs
      // the delta without re-flowing children mid-animation.
      className="hidden shrink-0 flex-col border-r border-border/40 bg-background/30 backdrop-blur-sm transition-[width] duration-200 ease-out md:flex"
      style={{ width: collapsed ? RAIL_WIDTH_COLLAPSED : RAIL_WIDTH_EXPANDED }}
      aria-label="Global navigation"
    >
      {/* Brand spot */}
      <div
        className={cn(
          'flex h-12 shrink-0 items-center border-b border-border/40 px-3',
          collapsed && 'justify-center px-0',
        )}
      >
        {collapsed ? (
          <div className="flex h-7 w-7 items-center justify-center rounded-md bg-foreground/90 text-[12px] font-semibold leading-none tracking-tight text-background">
            r
          </div>
        ) : (
          <span className="text-[15px] font-semibold leading-none tracking-tight text-foreground">
            rara
          </span>
        )}
      </div>

      {/* Nav items */}
      <nav className="flex flex-1 flex-col gap-0.5 overflow-y-auto p-2">
        {NAV_ITEMS.map((item) => (
          <RailNavLink key={item.to} item={item} collapsed={collapsed} />
        ))}
      </nav>

      {/* Bottom strip — Settings + status dot + collapse toggle */}
      <div
        className={cn(
          'flex shrink-0 flex-col gap-1 border-t border-border/40 p-2',
          collapsed && 'items-center',
        )}
      >
        <RailButton
          icon={Settings}
          label="Settings"
          collapsed={collapsed}
          onClick={() => openSettings()}
        />
        <div
          className={cn(
            'flex items-center',
            collapsed ? 'justify-center px-1.5 py-1.5' : 'gap-2 px-2 py-1.5',
          )}
        >
          <ConnectionDot />
          {!collapsed && <ConnectionLabel />}
        </div>
        <Button
          variant="ghost"
          size="icon"
          className={cn(
            'h-7 w-7 self-end text-muted-foreground transition-transform hover:text-foreground active:scale-[0.96]',
            collapsed && 'self-center',
          )}
          onClick={() => setCollapsed((v) => !v)}
          aria-label={collapsed ? 'Expand navigation' : 'Collapse navigation'}
          title={collapsed ? 'Expand navigation' : 'Collapse navigation'}
        >
          {collapsed ? <PanelLeft className="h-4 w-4" /> : <PanelLeftClose className="h-4 w-4" />}
        </Button>
      </div>
    </aside>
  );
}

function RailNavLink({ item, collapsed }: { item: NavItem; collapsed: boolean }) {
  const Icon = item.icon;
  return (
    <NavLink
      to={item.to}
      end={!item.matchPrefix}
      className={({ isActive }) =>
        cn(
          'group flex items-center gap-2 rounded-md px-2 py-1.5 text-sm transition-colors',
          'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-1',
          'hover:bg-foreground/5',
          isActive
            ? 'bg-foreground/8 text-foreground'
            : 'text-muted-foreground hover:text-foreground',
          collapsed && 'justify-center px-0',
        )
      }
      title={collapsed ? item.label : undefined}
      aria-label={collapsed ? item.label : undefined}
    >
      <Icon className="h-4 w-4 shrink-0" />
      {!collapsed && <span className="truncate">{item.label}</span>}
    </NavLink>
  );
}

function RailButton({
  icon: Icon,
  label,
  collapsed,
  onClick,
}: {
  icon: React.ComponentType<{ className?: string }>;
  label: string;
  collapsed: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        'group flex items-center gap-2 rounded-md px-2 py-1.5 text-sm text-muted-foreground transition-colors',
        'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-1',
        'hover:bg-foreground/5 hover:text-foreground',
        collapsed && 'justify-center px-0',
      )}
      title={collapsed ? label : undefined}
      aria-label={collapsed ? label : undefined}
    >
      <Icon className="h-4 w-4 shrink-0" />
      {!collapsed && <span className="truncate">{label}</span>}
    </button>
  );
}

function ConnectionDot() {
  const { isOnline, isChecking } = useServerStatus();
  const tooltip = isChecking ? 'Checking…' : isOnline ? 'Connected' : 'Disconnected';
  return (
    <span
      className={cn(
        'inline-block h-2 w-2 rounded-full transition-colors',
        isChecking ? 'bg-muted-foreground/50' : isOnline ? 'bg-green-500' : 'bg-red-500',
      )}
      title={tooltip}
      aria-label={`Backend: ${tooltip}`}
      role="status"
    />
  );
}

function ConnectionLabel() {
  const { isOnline, isChecking } = useServerStatus();
  // Rail-only label — kept terse so the rail width can stay slim. The
  // top bar no longer carries the "Connected" word per the #2059
  // restructure; this label is the only place it survives, gated to the
  // expanded rail.
  if (isChecking) {
    return <span className="text-xs text-muted-foreground/70">Checking…</span>;
  }
  return (
    <span className="text-xs text-muted-foreground">{isOnline ? 'Connected' : 'Disconnected'}</span>
  );
}
