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
import { Plus, RefreshCw } from 'lucide-react';
import { useEffect } from 'react';

import { api } from '@/api/client';
import type { ChatSession } from '@/api/types';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';

/** React-query key for the session list — exported so other panels can
 *  invalidate it after creating / mutating sessions. */
export const SESSIONS_QUERY_KEY = ['topology', 'chat-sessions'] as const;

/** Page size for the picker. The view is a sidebar, not a session
 *  browser — 50 keeps the rail usable without paging. */
const SESSION_LIMIT = 50;

/** Background poll cadence. The topology WS already covers in-flight
 *  events; this only catches sessions created in another tab. */
const REFETCH_MS = 30_000;

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
 */
export function SessionPicker({ activeSessionKey, onSelect, onAutoSelect }: SessionPickerProps) {
  const queryClient = useQueryClient();

  const sessionsQuery = useQuery({
    queryKey: SESSIONS_QUERY_KEY,
    queryFn: () => api.get<ChatSession[]>(`/api/v1/chat/sessions?limit=${SESSION_LIMIT}&offset=0`),
    refetchInterval: REFETCH_MS,
  });

  const createMutation = useMutation({
    mutationFn: () => api.post<ChatSession>('/api/v1/chat/sessions', { title: 'New session' }),
    onSuccess: (created) => {
      // Optimistically prepend so the new session is selectable before
      // the next refetch — matches what the user expects after clicking
      // a Create button.
      queryClient.setQueryData<ChatSession[]>(SESSIONS_QUERY_KEY, (prev) =>
        prev ? [created, ...prev] : [created],
      );
      onSelect(created.key);
    },
  });

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
            className="h-6 w-6"
            title="Refresh"
            onClick={() => void sessionsQuery.refetch()}
            disabled={sessionsQuery.isFetching}
          >
            <RefreshCw className={cn('h-3 w-3', sessionsQuery.isFetching && 'animate-spin')} />
          </Button>
          <Button
            size="icon"
            variant="ghost"
            className="h-6 w-6"
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
            {sessions.map((session) => (
              <SessionPickerItem
                key={session.key}
                session={session}
                active={session.key === activeSessionKey}
                onSelect={onSelect}
              />
            ))}
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
}

function SessionPickerItem({ session, active, onSelect }: SessionPickerItemProps) {
  const title = session.title?.trim() || 'Untitled session';
  const meta = `${formatRelativeTime(session.updated_at)} · ${session.message_count} msg`;

  return (
    <li>
      <button
        type="button"
        onClick={() => onSelect(session.key)}
        className={cn(
          'flex w-full flex-col items-start gap-0.5 rounded-md border px-2.5 py-2 text-left transition-colors',
          active
            ? 'border-accent bg-accent/10 text-foreground'
            : 'border-transparent text-foreground hover:bg-accent/5',
        )}
      >
        <span className="line-clamp-1 text-sm font-medium leading-tight">{title}</span>
        <span className="text-[11px] text-muted-foreground">{meta}</span>
      </button>
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
