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

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Archive, ArchiveRestore, EyeOff, Plus, RefreshCw } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';

import { api } from '@/api/client';
import { updateSessionStatus } from '@/api/sessions';
import type { ChatSession } from '@/api/types';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';

/** React-query key for the session list — exported so other panels can
 *  invalidate it after creating / mutating sessions. The query is
 *  partitioned by `showArchived` so toggling between the two views does
 *  not poison the cache. */
export const SESSIONS_QUERY_KEY = ['topology', 'chat-sessions'] as const;

/** Page size for the picker. The view is a sidebar, not a session
 *  browser — 50 keeps the rail usable without paging. */
const SESSION_LIMIT = 50;

/** Background poll cadence. The topology WS already covers in-flight
 *  events; this only catches sessions created in another tab. */
const REFETCH_MS = 30_000;

/** localStorage key for the "Show archived" toggle (issue #2043
 *  Decision 7). Stable string so a future migration can detect prior
 *  user preference. */
export const SHOW_ARCHIVED_STORAGE_KEY = 'rara.sidebar.showArchived';

/** Read the persisted "show archived" flag, defaulting to `false`.
 *  Defensive against older browsers / SSR contexts that lack
 *  `localStorage`. */
function readShowArchived(): boolean {
  if (typeof window === 'undefined') return false;
  try {
    return window.localStorage.getItem(SHOW_ARCHIVED_STORAGE_KEY) === 'true';
  } catch {
    return false;
  }
}

/** Persist the toggle. Failures (quota / disabled storage) are
 *  swallowed — the toggle still works for the current session. */
function writeShowArchived(value: boolean) {
  if (typeof window === 'undefined') return;
  try {
    window.localStorage.setItem(SHOW_ARCHIVED_STORAGE_KEY, value ? 'true' : 'false');
  } catch {
    // ignore — the in-memory state is the source of truth for this session
  }
}

export interface SessionPickerProps {
  /** Currently selected session key, or `null` when no selection yet. */
  activeSessionKey: string | null;
  /** Called when the user clicks a session card. */
  onSelect: (key: string) => void;
  /**
   * Called once after the first successful fetch when no session is
   * currently selected, with the most-recent session's key. The shell
   * uses this to auto-redirect `/topology` → `/topology/{key}`.
   */
  onAutoSelect?: (key: string) => void;
}

/**
 * Left-rail session picker for the topology shell. Lists the most
 * recently updated chat sessions and lets the user click into one
 * (or create a new one) instead of pasting a session UUID into a text
 * input — the experience #1999 reviewers complained about.
 *
 * Issue #2043: archived sessions are hidden by default behind the
 * "Show archived" toggle in the rail header. Each row carries an
 * archive / unarchive button (disabled on the active session so the
 * post-archive "what now?" question never surfaces).
 */
export function SessionPicker({ activeSessionKey, onSelect, onAutoSelect }: SessionPickerProps) {
  const queryClient = useQueryClient();
  const [showArchived, setShowArchived] = useState<boolean>(readShowArchived);

  // The query key carries the toggle so the rail does not flash stale
  // results when the user flips the visibility — react-query treats
  // them as two distinct caches.
  const queryKey = [...SESSIONS_QUERY_KEY, showArchived ? 'all' : 'active'] as const;
  const statusParam = showArchived ? 'all' : 'active';

  const sessionsQuery = useQuery({
    queryKey,
    queryFn: () =>
      api.get<ChatSession[]>(
        `/api/v1/chat/sessions?limit=${SESSION_LIMIT}&offset=0&status=${statusParam}`,
      ),
    refetchInterval: REFETCH_MS,
  });

  const createMutation = useMutation({
    mutationFn: () => api.post<ChatSession>('/api/v1/chat/sessions', { title: 'New session' }),
    onSuccess: (created) => {
      // Optimistically prepend so the new session is selectable before
      // the next refetch — matches what the user expects after clicking
      // a Create button.
      queryClient.setQueryData<ChatSession[]>(queryKey, (prev) =>
        prev ? [created, ...prev] : [created],
      );
      onSelect(created.key);
    },
  });

  const archiveMutation = useMutation({
    mutationFn: ({ key, status }: { key: string; status: 'active' | 'archived' }) =>
      updateSessionStatus(key, status),
    onSuccess: (_updated, vars) => {
      // The default-Active view drops archived rows; the all-view
      // keeps both. Either way, an explicit invalidate is the smallest
      // correct behaviour — the optimistic prune below covers the
      // archive-from-active case so the row disappears immediately
      // without waiting for the refetch round-trip.
      if (!showArchived && vars.status === 'archived') {
        queryClient.setQueryData<ChatSession[]>(queryKey, (prev) =>
          prev ? prev.filter((s) => s.key !== vars.key) : prev,
        );
      }
      void queryClient.invalidateQueries({ queryKey: SESSIONS_QUERY_KEY });
    },
  });

  const handleToggleShowArchived = useCallback(() => {
    setShowArchived((prev) => {
      const next = !prev;
      writeShowArchived(next);
      return next;
    });
  }, []);

  const sessions = sessionsQuery.data ?? [];
  const firstKey = sessions[0]?.key;

  // Auto-select the most-recent session when the URL has no key. The
  // effect re-fires only when the relevant inputs change, and the guard
  // on `activeSessionKey === null` prevents it from clobbering a user
  // selection mid-session.
  useEffect(() => {
    if (!onAutoSelect) return;
    if (activeSessionKey !== null) return;
    if (!firstKey) return;
    onAutoSelect(firstKey);
  }, [activeSessionKey, firstKey, onAutoSelect]);

  return (
    <div className="group flex h-full flex-col">
      <div className="flex items-center justify-between border-b border-border px-3 py-2">
        <span className="text-xs font-medium text-muted-foreground">Sessions</span>
        <div className="flex items-center gap-1">
          <Button
            size="icon"
            variant="ghost"
            className={cn(
              'h-6 w-6 transition-transform active:scale-[0.96]',
              showArchived && 'bg-accent/30 text-foreground',
            )}
            title={showArchived ? 'Hide archived' : 'Show archived'}
            aria-pressed={showArchived}
            onClick={handleToggleShowArchived}
          >
            {showArchived ? <ArchiveRestore className="h-3 w-3" /> : <EyeOff className="h-3 w-3" />}
          </Button>
          <Button
            size="icon"
            variant="ghost"
            className="h-6 w-6 transition-transform active:scale-[0.96]"
            title="Refresh"
            onClick={() => void sessionsQuery.refetch()}
            disabled={sessionsQuery.isFetching}
          >
            <RefreshCw className={cn('h-3 w-3', sessionsQuery.isFetching && 'animate-spin')} />
          </Button>
          <Button
            size="icon"
            variant="ghost"
            className="h-6 w-6 transition-transform active:scale-[0.96]"
            title="New session"
            onClick={() => createMutation.mutate()}
            disabled={createMutation.isPending}
          >
            <Plus className="h-3.5 w-3.5" />
          </Button>
        </div>
      </div>

      <div className="scrollbar-hover flex-1 overflow-y-auto">
        {sessionsQuery.isLoading ? (
          <SessionPickerEmpty label="Loading sessions…" />
        ) : sessionsQuery.isError ? (
          <SessionPickerEmpty
            label="Failed to load sessions"
            action={{ label: 'Retry', onClick: () => void sessionsQuery.refetch() }}
          />
        ) : sessions.length === 0 ? (
          <SessionPickerEmpty
            label="No sessions yet"
            action={{
              label: createMutation.isPending ? 'Creating…' : 'Create session',
              onClick: () => createMutation.mutate(),
              disabled: createMutation.isPending,
            }}
          />
        ) : (
          <ul className="flex flex-col gap-px p-1.5">
            {sessions.map((session) => {
              const isActiveSelected = session.key === activeSessionKey;
              const status = session.status ?? 'active';
              return (
                <SessionPickerItem
                  key={session.key}
                  session={session}
                  active={isActiveSelected}
                  onSelect={onSelect}
                  onArchiveToggle={(targetStatus) =>
                    archiveMutation.mutate({ key: session.key, status: targetStatus })
                  }
                  archiveDisabled={isActiveSelected || archiveMutation.isPending}
                  archiveDisabledReason={
                    isActiveSelected ? 'Switch to another session first' : undefined
                  }
                  status={status}
                />
              );
            })}
          </ul>
        )}
      </div>
    </div>
  );
}

interface SessionPickerItemProps {
  session: ChatSession;
  active: boolean;
  onSelect: (key: string) => void;
  /** Fires the archive / unarchive PATCH for this row. */
  onArchiveToggle: (status: 'active' | 'archived') => void;
  /** Block the archive button (active-row case + in-flight mutation). */
  archiveDisabled: boolean;
  /** Tooltip shown on the disabled archive button. `undefined` when the
   *  button is enabled. */
  archiveDisabledReason: string | undefined;
  /** Materialised status — `'active'` when the wire payload omits the
   *  field (back-compat with payloads predating issue #2043). */
  status: 'active' | 'archived';
}

function SessionPickerItem({
  session,
  active,
  onSelect,
  onArchiveToggle,
  archiveDisabled,
  archiveDisabledReason,
  status,
}: SessionPickerItemProps) {
  const title = session.title?.trim() || 'Untitled session';
  const meta = `${formatRelativeTime(session.updated_at)} · ${session.message_count} msg`;
  const isArchived = status === 'archived';
  const targetStatus: 'active' | 'archived' = isArchived ? 'active' : 'archived';
  const buttonTitle =
    archiveDisabledReason ?? (isArchived ? 'Unarchive session' : 'Archive session');

  return (
    <li className="group/row relative">
      <button
        type="button"
        onClick={() => onSelect(session.key)}
        className={cn(
          // `rounded-lg` on the row + `px-2.5 py-2` padding lands inside
          // a `rounded-xl` parent container at the standard 4px concentric
          // delta (principle #1). Press-scale per #12, transition list
          // explicit per #14 (no `transition: all`).
          'flex w-full flex-col items-start gap-0.5 rounded-lg border px-2.5 py-2 text-left',
          'transition-[colors,transform] active:scale-[0.98]',
          active
            ? 'border-accent bg-accent/10 text-foreground'
            : 'border-transparent text-foreground hover:bg-accent/5',
          isArchived && 'opacity-60',
        )}
      >
        <span className="line-clamp-1 text-sm font-medium leading-tight">{title}</span>
        {/* `tabular-nums` (#9) — the message count ticks during a live
            session and would otherwise reflow the row width. */}
        <span className="text-[11px] tabular-nums text-muted-foreground">{meta}</span>
      </button>
      <div className="absolute right-1 top-1 opacity-0 transition-opacity group-hover/row:opacity-100 focus-within:opacity-100">
        <Button
          size="icon"
          variant="ghost"
          className="h-6 w-6"
          title={buttonTitle}
          aria-label={buttonTitle}
          disabled={archiveDisabled}
          onClick={(event) => {
            event.stopPropagation();
            onArchiveToggle(targetStatus);
          }}
        >
          {isArchived ? <ArchiveRestore className="h-3 w-3" /> : <Archive className="h-3 w-3" />}
        </Button>
      </div>
    </li>
  );
}

interface SessionPickerEmptyProps {
  label: string;
  action?: { label: string; onClick: () => void; disabled?: boolean };
}

function SessionPickerEmpty({ label, action }: SessionPickerEmptyProps) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-2 px-4 py-8 text-center">
      <span className="text-xs text-muted-foreground">{label}</span>
      {action && (
        <Button
          size="sm"
          variant="outline"
          className="h-7 gap-1 text-xs"
          onClick={action.onClick}
          disabled={action.disabled}
        >
          <Plus className="h-3 w-3" />
          {action.label}
        </Button>
      )}
    </div>
  );
}

/**
 * Compact relative-time label (`now`, `5m ago`, `2h ago`, `3d ago`,
 * falling back to a date for older entries). Pure — no `Intl`
 * RelativeTimeFormat to keep the bundle lean and the output deterministic
 * across locales (the picker is a developer surface, not user-facing
 * copy).
 */
function formatRelativeTime(iso: string): string {
  const then = Date.parse(iso);
  if (Number.isNaN(then)) return iso;
  const deltaSec = Math.max(0, Math.round((Date.now() - then) / 1000));
  if (deltaSec < 45) return 'now';
  if (deltaSec < 3600) return `${Math.round(deltaSec / 60)}m ago`;
  if (deltaSec < 86_400) return `${Math.round(deltaSec / 3600)}h ago`;
  if (deltaSec < 7 * 86_400) return `${Math.round(deltaSec / 86_400)}d ago`;
  return new Date(then).toISOString().slice(0, 10);
}
