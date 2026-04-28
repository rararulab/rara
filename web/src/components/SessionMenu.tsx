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

import { Check, Loader2, MoreHorizontal, RefreshCw } from 'lucide-react';
import { useState } from 'react';

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from './ui/dropdown-menu';

import { api } from '@/api/client';
import type { ChatSession } from '@/api/types';

interface SessionMenuProps {
  /** Session this menu acts on. Only `key` is required at call time. */
  sessionKey: string;
  /** Optional accessible label suffix (e.g. session title) for the trigger. */
  ariaLabel?: string;
  /** Called with the refreshed session after a successful title regeneration
   *  so the caller can update its local cache (e.g. the page-header title). */
  onRegenerated?: (session: ChatSession) => void;
}

/**
 * Copy `text` to the clipboard via the legacy `document.execCommand('copy')`
 * path. Used as a fallback when `navigator.clipboard` is unavailable, which
 * happens whenever the page is served from a non-secure context (e.g. a LAN
 * IP without TLS). Returns `true` on success.
 *
 * The textarea is positioned off-screen and marked readonly so the operation
 * does not flash UI, scroll the page, or pop the keyboard on iOS.
 */
function copyTextFallback(text: string): boolean {
  if (typeof document === 'undefined') return false;
  const textarea = document.createElement('textarea');
  textarea.value = text;
  textarea.setAttribute('readonly', '');
  textarea.style.position = 'fixed';
  textarea.style.top = '-9999px';
  textarea.style.left = '0';
  textarea.style.opacity = '0';
  document.body.appendChild(textarea);
  try {
    textarea.select();
    return document.execCommand('copy');
  } catch {
    return false;
  } finally {
    document.body.removeChild(textarea);
  }
}

/**
 * Hover-revealed `⋯` menu attached to a session row or the chat-page header
 * title. Exposes two actions: copy the session id to the clipboard, and
 * regenerate the session title via the backend.
 */
export function SessionMenu({ sessionKey, ariaLabel, onRegenerated }: SessionMenuProps) {
  const [copied, setCopied] = useState(false);
  const [regenerating, setRegenerating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleCopy = async () => {
    // `navigator.clipboard` is only defined in secure contexts (https /
    // localhost). When the dev server is reached over a LAN IP it is
    // `undefined`, so optional-chain the call and fall back to the legacy
    // `document.execCommand('copy')` path before surfacing an error.
    try {
      if (navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(sessionKey);
        setCopied(true);
        setTimeout(() => setCopied(false), 1200);
        return;
      }
    } catch {
      // fall through to the execCommand fallback
    }

    if (copyTextFallback(sessionKey)) {
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
      return;
    }

    setError('复制失败');
    setTimeout(() => setError(null), 1500);
  };

  const handleRegenerate = async (e: Event) => {
    // Keep the menu open while the request is in flight so the loading
    // state is visible; Radix would otherwise close on selection.
    e.preventDefault();
    if (regenerating) return;
    setRegenerating(true);
    setError(null);
    try {
      const updated = await api.post<ChatSession>(
        `/api/v1/chat/sessions/${encodeURIComponent(sessionKey)}/regenerate-title`,
      );
      onRegenerated?.(updated);
    } catch {
      setError('重新生成失败');
      setTimeout(() => setError(null), 2000);
    } finally {
      setRegenerating(false);
    }
  };

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          aria-label={ariaLabel ? `更多操作 — ${ariaLabel}` : '更多操作'}
          title="更多操作"
          onClick={(e) => e.stopPropagation()}
          className="shrink-0 cursor-pointer rounded p-1 text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-foreground"
        >
          <MoreHorizontal className="h-3.5 w-3.5" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" onClick={(e) => e.stopPropagation()}>
        <DropdownMenuItem onSelect={() => void handleCopy()}>
          {copied ? (
            <Check className="h-3.5 w-3.5 text-green-600" />
          ) : (
            <Check className="h-3.5 w-3.5 opacity-0" />
          )}
          <span>{copied ? '已复制' : '复制 session id'}</span>
        </DropdownMenuItem>
        <DropdownMenuItem disabled={regenerating} onSelect={(e) => void handleRegenerate(e)}>
          {regenerating ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : (
            <RefreshCw className="h-3.5 w-3.5" />
          )}
          <span>{regenerating ? '生成中…' : '重新生成标题'}</span>
        </DropdownMenuItem>
        {error && (
          <div className="px-2 py-1 text-[11px] text-destructive" role="alert">
            {error}
          </div>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
