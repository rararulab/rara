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

import { PanelLeftClose, PanelLeft, Plus, Search, Settings, Trash2 } from 'lucide-react';
import { useEffect, useState } from 'react';

import { SidebarRunHistory } from './SidebarRunHistory';

import { api } from '@/api/client';
import type { ChatSession } from '@/api/types';
import { pickSessionFallback } from '@/lib/session-fallback';
import { cn } from '@/lib/utils';

const COLLAPSED_STORAGE_KEY = 'rara.sidebar.collapsed';

interface ChatSidebarProps {
  activeSessionKey: string | undefined;
  onSelect: (session: ChatSession) => void;
  onNewSession: () => void;
  onOpenSearch: () => void;
  onOpenSettings: () => void;
  /** Called after a session is deleted. `fallback` is the next
   * session the caller should switch to when the deleted row was
   * the active one, or `null` when no sessions are left. */
  onDeleteSession: (key: string, fallback: ChatSession | null) => void;
  /** Bump this from the parent to force a session-list refetch (e.g. after
   * creating a new session or receiving the first message of a fresh one). */
  refreshKey: number;
}

function stripForPreview(text: string): string {
  return text.replace(/<think>[\s\S]*?<\/think>\s*/g, '').trim();
}

function formatRelativeDate(iso: string): string {
  const diff = Date.now() - new Date(iso).getTime();
  const days = Math.floor(diff / 86_400_000);
  if (days === 0) return '今天';
  if (days === 1) return '昨天';
  if (days < 7) return `${days} 天前`;
  return new Date(iso).toLocaleDateString();
}

/**
 * Persistent left-hand sidebar for the chat page: new-session button,
 * settings entry, and a scrollable history list. Collapsible to an
 * icon-only rail; the collapsed state is persisted to `localStorage`
 * so the choice survives reloads.
 */
export function ChatSidebar({
  activeSessionKey,
  onSelect,
  onNewSession,
  onOpenSearch,
  onOpenSettings,
  onDeleteSession,
  refreshKey,
}: ChatSidebarProps) {
  const [collapsed, setCollapsed] = useState<boolean>(() => {
    try {
      return localStorage.getItem(COLLAPSED_STORAGE_KEY) === '1';
    } catch {
      return false;
    }
  });
  const [sessions, setSessions] = useState<ChatSession[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let alive = true;
    setLoading(true);
    api
      .get<ChatSession[]>('/api/v1/chat/sessions?limit=100&offset=0')
      .then((list) => {
        if (alive) setSessions(list);
      })
      .catch(() => {
        if (alive) setSessions([]);
      })
      .finally(() => {
        if (alive) setLoading(false);
      });
    return () => {
      alive = false;
    };
  }, [refreshKey]);

  const toggleCollapsed = () => {
    setCollapsed((prev) => {
      const next = !prev;
      try {
        localStorage.setItem(COLLAPSED_STORAGE_KEY, next ? '1' : '0');
      } catch {
        /* ignore */
      }
      return next;
    });
  };

  const handleDelete = async (key: string, e: React.MouseEvent) => {
    e.stopPropagation();
    if (!confirm('删除这个会话？')) return;
    try {
      await api.del(`/api/v1/chat/sessions/${encodeURIComponent(key)}`);
      // Pick the neighbour *before* the list mutation so the
      // parent can switch into a still-cached row rather than
      // spinning up a fresh session.
      const fallback = pickSessionFallback(sessions, key);
      setSessions((prev) => prev.filter((s) => s.key !== key));
      onDeleteSession(key, fallback);
    } catch {
      /* ignore */
    }
  };

  return (
    <aside
      className={cn(
        'flex h-screen shrink-0 flex-col border-r border-border bg-canvas transition-[width] duration-200 ease-out',
        collapsed ? 'w-14' : 'w-64',
      )}
    >
      {/* Top: logo + collapse toggle */}
      <div
        className={cn(
          'flex h-12 shrink-0 items-center border-b border-border/40',
          collapsed ? 'justify-center' : 'justify-between px-3',
        )}
      >
        {!collapsed && (
          <span className="text-[20px] font-semibold leading-none tracking-tight text-foreground">
            rara
          </span>
        )}
        <button
          type="button"
          onClick={toggleCollapsed}
          className="flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground hover:bg-secondary/60 hover:text-foreground transition-colors cursor-pointer"
          title={collapsed ? '展开边栏' : '折叠边栏'}
        >
          {collapsed ? <PanelLeft className="h-4 w-4" /> : <PanelLeftClose className="h-4 w-4" />}
        </button>
      </div>

      {/* Actions: new session + settings */}
      <div
        className={cn(
          'flex shrink-0 flex-col gap-1',
          collapsed ? 'items-center py-2' : 'px-2 py-2',
        )}
      >
        <button
          type="button"
          onClick={onNewSession}
          className={cn(
            'flex h-9 items-center rounded-md text-sm font-medium text-foreground transition-colors cursor-pointer hover:bg-secondary/60',
            collapsed ? 'w-9 justify-center' : 'w-full gap-2 px-3',
          )}
          title="新建会话"
        >
          <Plus className="h-4 w-4 shrink-0 text-brand" />
          {!collapsed && <span className="truncate">新建会话</span>}
        </button>
        <button
          type="button"
          onClick={onOpenSearch}
          className={cn(
            'flex h-9 items-center rounded-md text-sm text-muted-foreground transition-colors cursor-pointer hover:bg-secondary/60 hover:text-foreground',
            collapsed ? 'w-9 justify-center' : 'w-full gap-2 px-3',
          )}
          title="搜索会话 (⌘K)"
        >
          <Search className="h-4 w-4 shrink-0" />
          {!collapsed && <span className="truncate">搜索会话</span>}
        </button>
        <button
          type="button"
          onClick={onOpenSettings}
          className={cn(
            'flex h-9 items-center rounded-md text-sm text-muted-foreground transition-colors cursor-pointer hover:bg-secondary/60 hover:text-foreground',
            collapsed ? 'w-9 justify-center' : 'w-full gap-2 px-3',
          )}
          title="设置"
        >
          <Settings className="h-4 w-4 shrink-0" />
          {!collapsed && <span className="truncate">设置</span>}
        </button>
      </div>

      {/* History list */}
      {!collapsed && (
        <>
          <div className="mb-1 mt-4 shrink-0 px-4 text-[11px] font-medium uppercase tracking-wider text-muted-foreground/70">
            历史会话
          </div>
          <div className="min-h-0 flex-1 overflow-y-auto">
            {loading ? (
              <div className="py-6 text-center text-xs text-muted-foreground">加载中…</div>
            ) : sessions.length === 0 ? (
              <div className="py-6 text-center text-xs text-muted-foreground">暂无会话</div>
            ) : (
              sessions.map((s) => (
                <div
                  key={s.key}
                  className={cn(
                    'group mx-2 flex items-start gap-2 rounded-md border-l-2 text-sm transition-colors',
                    s.key === activeSessionKey
                      ? 'border-brand bg-secondary/70 text-foreground'
                      : 'border-transparent text-foreground/80 hover:bg-secondary/50 hover:text-foreground',
                  )}
                >
                  <button
                    type="button"
                    onClick={() => onSelect(s)}
                    className="min-w-0 flex-1 cursor-pointer text-left px-2 py-1.5 bg-transparent"
                  >
                    <div className="truncate text-[13px] leading-tight">
                      {stripForPreview(s.title || s.preview || '新对话')}
                    </div>
                    <div className="mt-0.5 truncate text-[11px] text-muted-foreground/80">
                      {formatRelativeDate(s.updated_at)}
                    </div>
                  </button>
                  <button
                    type="button"
                    onClick={(e) => handleDelete(s.key, e)}
                    aria-label={`删除 ${s.title ?? '会话'}`}
                    className="shrink-0 rounded p-1 mr-1 mt-1 text-muted-foreground/0 transition-[color,opacity] hover:bg-destructive/10 hover:text-destructive group-hover:text-muted-foreground group-hover:opacity-100"
                    title="删除"
                  >
                    <Trash2 className="h-3 w-3" />
                  </button>
                </div>
              ))
            )}
          </div>
          <SidebarRunHistory activeSessionKey={activeSessionKey} />
        </>
      )}
    </aside>
  );
}
