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

import { useEffect, useState } from 'react';

import { searchSessions, type SessionSearchHit } from '@/api/sessions';
import type { ChatSession } from '@/api/types';
import {
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from '@/components/ui/command';
import { cn } from '@/lib/utils';

interface SessionSearchDialogProps {
  open: boolean;
  onOpenChange: (value: boolean) => void;
  /** Called with the session key the user picked. Parent wires this to the
   *  same session-switch flow used by sidebar clicks. */
  onSelect: (sessionKey: string) => void;
  /** Recent sessions used to populate the empty-query list. */
  recentSessions: ChatSession[];
}

const DEBOUNCE_MS = 250;
const SEARCH_LIMIT = 20;
const RECENT_LIMIT = 10;

function formatRelative(ms: number): string {
  const diff = Date.now() - ms;
  const days = Math.floor(diff / 86_400_000);
  if (days <= 0) return '今天';
  if (days === 1) return '昨天';
  if (days < 7) return `${days} 天前`;
  return new Date(ms).toLocaleDateString();
}

function formatRelativeIso(iso: string): string {
  return formatRelative(new Date(iso).getTime());
}

function roleLabel(role: SessionSearchHit['role']): string {
  switch (role) {
    case 'user':
      return '你';
    case 'assistant':
      return 'rara';
    default:
      return role;
  }
}

// Styles the `<mark>` spans emitted by the backend so matched terms
// stand out. Kept co-located here rather than in `command.tsx` because
// the highlighting is specific to this dialog's snippet rendering.
const SNIPPET_CLASS =
  'text-xs leading-snug text-muted-foreground [&_mark]:bg-yellow-200/60 [&_mark]:text-foreground dark:[&_mark]:bg-yellow-500/25 [&_mark]:rounded-sm [&_mark]:px-0.5';

/**
 * Cmd+K session search palette.
 *
 * Empty query → recent sessions.
 * Non-empty query → debounced search against the backend (250ms), with a
 * loading row in the debounce/network window and an empty-state message
 * when the API returns no hits.
 *
 * Trust boundary: `SessionSearchHit.snippet` is HTML produced server-side
 * — the backend escapes user text and only injects `<mark>…</mark>` for
 * the matched query term. We render it with `dangerouslySetInnerHTML`
 * because any other path would strip those marks. Do NOT feed arbitrary
 * text through this code path.
 */
export function SessionSearchDialog({
  open,
  onOpenChange,
  onSelect,
  recentSessions,
}: SessionSearchDialogProps) {
  const [query, setQuery] = useState('');
  const [hits, setHits] = useState<SessionSearchHit[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Reset the query whenever the dialog closes so the next open starts
  // from a clean slate — otherwise stale results flash on reopen.
  useEffect(() => {
    if (!open) {
      setQuery('');
      setHits([]);
      setLoading(false);
      setError(null);
    }
  }, [open]);

  // Debounced search. We skip the fetch entirely on empty queries so the
  // recents list renders without any network chatter.
  useEffect(() => {
    if (!open) return;
    const trimmed = query.trim();
    if (!trimmed) {
      setHits([]);
      setLoading(false);
      setError(null);
      return;
    }
    setLoading(true);
    setError(null);
    const controller = new AbortController();
    const timer = setTimeout(() => {
      searchSessions(trimmed, SEARCH_LIMIT, { signal: controller.signal })
        .then((res) => {
          setHits(res);
          setLoading(false);
        })
        .catch((e: unknown) => {
          if (controller.signal.aborted) return;
          const msg = e instanceof Error ? e.message : String(e);
          setError(msg);
          setLoading(false);
        });
    }, DEBOUNCE_MS);
    return () => {
      clearTimeout(timer);
      controller.abort();
    };
  }, [query, open]);

  const handlePick = (sessionKey: string) => {
    onSelect(sessionKey);
    onOpenChange(false);
  };

  const trimmed = query.trim();
  const showRecents = trimmed.length === 0;

  return (
    <CommandDialog open={open} onOpenChange={onOpenChange}>
      <CommandInput placeholder="搜索会话 (Cmd+K)…" value={query} onValueChange={setQuery} />
      <CommandList>
        {showRecents ? (
          recentSessions.length === 0 ? (
            <CommandEmpty>暂无会话</CommandEmpty>
          ) : (
            <CommandGroup heading="最近会话">
              {recentSessions.slice(0, RECENT_LIMIT).map((s) => (
                <CommandItem
                  key={s.key}
                  value={`recent:${s.key}:${s.title ?? ''}`}
                  onSelect={() => handlePick(s.key)}
                >
                  <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                    <div className="truncate text-sm font-medium text-foreground">
                      {s.title || s.preview || '新对话'}
                    </div>
                    <div className="text-xs text-muted-foreground">
                      {formatRelativeIso(s.updated_at)}
                    </div>
                  </div>
                </CommandItem>
              ))}
            </CommandGroup>
          )
        ) : loading ? (
          // Simple shimmer row while the debounce + fetch are in flight.
          <div className="space-y-2 p-3" aria-label="加载中" role="status">
            {[0, 1, 2].map((i) => (
              <div key={i} className="h-10 w-full animate-pulse rounded-md bg-muted/60" />
            ))}
          </div>
        ) : error ? (
          <CommandEmpty>
            <div className="text-xs text-destructive">搜索失败：{error}</div>
          </CommandEmpty>
        ) : hits.length === 0 ? (
          <CommandEmpty>没有匹配的会话</CommandEmpty>
        ) : (
          <CommandGroup heading="搜索结果">
            {hits.map((hit) => (
              <CommandItem
                key={`${hit.session_key}:${hit.seq}`}
                // cmdk filters by `value` — fall back to snippet text when
                // the title is empty so fuzzy selection still works.
                value={`${hit.session_key}:${hit.seq}:${hit.session_title}:${hit.snippet}`}
                onSelect={() => handlePick(hit.session_key)}
              >
                <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                  <div className="flex items-center gap-2">
                    <span className="truncate text-sm font-semibold text-foreground">
                      {hit.session_title || '新对话'}
                    </span>
                    <span
                      className={cn(
                        'shrink-0 rounded border border-border/60 px-1 py-0 text-[10px] uppercase tracking-wider text-muted-foreground',
                      )}
                    >
                      {roleLabel(hit.role)}
                    </span>
                    <span className="ml-auto shrink-0 text-[11px] text-muted-foreground">
                      {formatRelative(hit.timestamp_ms)}
                    </span>
                  </div>
                  {/* Trust boundary: see component doc. Snippet is backend-produced
                      HTML with only <mark> tags around matched text. */}
                  <div
                    className={SNIPPET_CLASS}
                    dangerouslySetInnerHTML={{ __html: hit.snippet }}
                  />
                </div>
              </CommandItem>
            ))}
          </CommandGroup>
        )}
      </CommandList>
    </CommandDialog>
  );
}
