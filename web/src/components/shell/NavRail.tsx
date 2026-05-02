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
 * Layout — top: small brand cue (only when collapsed). Middle: route
 * entries plus Settings as a peer nav item (no separator, no boxed
 * pill). Bottom: a quiet brand footer carrying the "rara" wordmark and
 * a small connection-status dot (tooltip on hover). The collapse
 * toggle is hover-revealed when the rail is expanded and rendered as
 * a single permanent affordance when collapsed, so the bottom no
 * longer ships a dedicated toggle strip.
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
      // `group` lets descendants opt into hover-revealed visibility
      // (`group-hover:opacity-100`) without each one needing its own
      // hover state. Width transitions only; main column has min-w-0
      // so it absorbs the delta without re-flowing children mid-animation.
      className="group/rail hidden shrink-0 flex-col border-r border-border/40 bg-background/30 backdrop-blur-sm transition-[width] duration-200 ease-out md:flex"
      style={{ width: collapsed ? RAIL_WIDTH_COLLAPSED : RAIL_WIDTH_EXPANDED }}
      aria-label="Global navigation"
    >
      {/* Top spot — brand glyph only when collapsed; expanded rail
          carries the wordmark in the footer instead, so the two never
          share weight. */}
      <div
        className={cn('flex h-12 shrink-0 items-center px-3', collapsed && 'justify-center px-0')}
      >
        {collapsed ? (
          <div className="flex h-7 w-7 items-center justify-center rounded-md bg-foreground/90 text-[12px] font-semibold leading-none tracking-tight text-background">
            r
          </div>
        ) : (
          // Expanded mode reserves the top strip's vertical space so
          // the nav list begins at a consistent baseline regardless of
          // collapse state, but renders nothing — the brand lives in
          // the footer.
          <span aria-hidden="true" />
        )}
      </div>

      {/* Nav list — Chat, Docs, and Settings share identical styling.
          Settings stays a button (it opens a modal, not a route) but
          is visually one of the nav entries. */}
      <nav className="flex flex-1 flex-col gap-0.5 overflow-y-auto p-2">
        {NAV_ITEMS.map((item) => (
          <RailNavLink key={item.to} item={item} collapsed={collapsed} />
        ))}
        <RailNavButton
          icon={Settings}
          label="Settings"
          collapsed={collapsed}
          onClick={() => openSettings()}
        />
      </nav>

      {/* Brand footer — wordmark + status dot when expanded; just the
          dot when collapsed. Collapse toggle is hover-revealed here
          (expanded) or rendered as the only element (collapsed) so the
          user always has a discoverable affordance. */}
      <div
        className={cn(
          'flex shrink-0 items-center px-3 py-2.5',
          collapsed ? 'flex-col gap-2' : 'justify-between gap-2',
        )}
      >
        {collapsed ? (
          <>
            <ConnectionDot />
            <CollapseToggle collapsed={collapsed} onClick={() => setCollapsed((v) => !v)} />
          </>
        ) : (
          <>
            <div className="flex min-w-0 items-center gap-2">
              <span className="text-[11px] font-medium tracking-wide text-muted-foreground/80">
                rara
              </span>
              <ConnectionDot />
            </div>
            <CollapseToggle collapsed={collapsed} onClick={() => setCollapsed((v) => !v)} />
          </>
        )}
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

/**
 * Button styled identically to `RailNavLink` for nav-equivalent
 * actions (e.g. opening a modal). Visual parity with the route entries
 * is the whole point — no separator, no boxed pill.
 */
function RailNavButton({
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
        // Fixed size so the footer baseline never shifts when the
        // status color flips between checking / online / offline.
        'inline-block h-1.5 w-1.5 shrink-0 rounded-full transition-colors',
        isChecking ? 'bg-muted-foreground/50' : isOnline ? 'bg-green-500' : 'bg-red-500',
      )}
      title={tooltip}
      aria-label={`Backend: ${tooltip}`}
      role="status"
    />
  );
}

/**
 * Collapse / expand affordance. Expanded rail hides it until the rail
 * is hovered or the button itself is focused (keyboard access stays
 * intact). Collapsed rail keeps it permanently visible — otherwise
 * the user has no way back out of the collapsed state.
 */
function CollapseToggle({ collapsed, onClick }: { collapsed: boolean; onClick: () => void }) {
  return (
    <Button
      variant="ghost"
      size="icon"
      className={cn(
        'h-6 w-6 shrink-0 text-muted-foreground/70 transition-[opacity,color] hover:text-foreground',
        // Hover-reveal only when expanded; collapsed mode needs a
        // permanent affordance so the user can expand back.
        !collapsed && 'opacity-0 focus-visible:opacity-100 group-hover/rail:opacity-100',
      )}
      onClick={onClick}
      aria-label={collapsed ? 'Expand navigation' : 'Collapse navigation'}
      title={collapsed ? 'Expand navigation' : 'Collapse navigation'}
    >
      {collapsed ? (
        <PanelLeft className="h-3.5 w-3.5" />
      ) : (
        <PanelLeftClose className="h-3.5 w-3.5" />
      )}
    </Button>
  );
}
