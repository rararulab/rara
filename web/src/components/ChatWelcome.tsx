/*
 * Copyright 2025 Rararulab
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 */

import { ArrowUpRight, Lightbulb, ListChecks, MessageSquare, Sparkles } from 'lucide-react';
import { useEffect, useState } from 'react';

import { api } from '@/api/client';
import type { ChatSession } from '@/api/types';

interface StarterPrompt {
  icon: typeof Sparkles;
  label: string;
  prompt: string;
}

// Curated starter prompts. Kept short; first line is the visible label,
// `prompt` is what gets dropped into the composer on click. Style mirrors
// ChatGPT/Claude empty states — concrete verbs, not marketing copy.
const STARTERS: StarterPrompt[] = [
  {
    icon: Sparkles,
    label: '帮我写一段',
    prompt: '帮我写一段关于 ',
  },
  {
    icon: Lightbulb,
    label: '解释一个概念',
    prompt: '请用通俗的语言解释 ',
  },
  {
    icon: ListChecks,
    label: '拆解任务',
    prompt: '帮我把这个目标拆成可执行的步骤：',
  },
  {
    icon: MessageSquare,
    label: '继续讨论',
    prompt: '我想继续讨论 ',
  },
];

interface ChatWelcomeProps {
  /** Open an existing session from the recent peek. */
  onSelectSession: (session: ChatSession) => void;
}

/**
 * Home empty-state panel. Rendered above pi-web-ui's empty message column
 * when the active session has no messages. Layout mirrors ChatGPT/Claude:
 * a calm greeting, four starter cards, and a peek at recent sessions —
 * no giant wordmark, no floating composer (the composer stays anchored
 * at the bottom of `<main>` in both empty and populated states).
 */
export function ChatWelcome({ onSelectSession }: ChatWelcomeProps) {
  const [recents, setRecents] = useState<ChatSession[]>([]);

  useEffect(() => {
    let alive = true;
    api
      .get<ChatSession[]>('/api/v1/chat/sessions?limit=4&offset=0')
      .then((list) => {
        if (alive) setRecents(list.filter((s) => s.message_count > 0));
      })
      .catch(() => {
        if (alive) setRecents([]);
      });
    return () => {
      alive = false;
    };
  }, []);

  // Prefill pi-web-ui's composer textarea. Pi-web-ui's editor reacts to
  // native `input` events to track the buffer, so dispatching one after
  // assignment keeps internal state in sync. We don't auto-send — the
  // user finishes the thought and presses Enter.
  const handleStarter = (prompt: string) => {
    const ta = document.querySelector<HTMLTextAreaElement>('textarea');
    if (!ta) return;
    ta.value = prompt;
    ta.dispatchEvent(new Event('input', { bubbles: true }));
    ta.focus();
    // Drop the caret at the end so the user can keep typing.
    const len = ta.value.length;
    ta.setSelectionRange(len, len);
  };

  return (
    <div className="pointer-events-auto mx-auto flex w-full max-w-3xl flex-col gap-8 px-6 pb-32 pt-16 sm:pt-24">
      <header className="flex flex-col gap-2">
        <h1 className="text-h1 text-foreground">你好，欢迎回来</h1>
        <p className="text-body-lg text-muted-foreground">今天想做点什么？</p>
      </header>

      <section className="flex flex-col gap-3">
        <h2 className="text-section-label text-muted-foreground">起步提示</h2>
        <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
          {STARTERS.map(({ icon: Icon, label, prompt }) => (
            <button
              key={label}
              type="button"
              onClick={() => handleStarter(prompt)}
              className="group flex items-center gap-3 rounded-xl border border-border/60 bg-background/60 px-4 py-3 text-left text-sm text-foreground transition-colors hover:border-border hover:bg-secondary/40"
            >
              <Icon className="h-4 w-4 shrink-0 text-muted-foreground transition-colors group-hover:text-foreground" />
              <span className="flex-1 truncate">{label}</span>
              <ArrowUpRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground/0 transition-opacity group-hover:text-muted-foreground group-hover:opacity-100" />
            </button>
          ))}
        </div>
      </section>

      {recents.length > 0 && (
        <section className="flex flex-col gap-3">
          <h2 className="text-section-label text-muted-foreground">最近会话</h2>
          <ul className="flex flex-col">
            {recents.map((s) => (
              <li key={s.key}>
                <button
                  type="button"
                  onClick={() => onSelectSession(s)}
                  className="group flex w-full items-center gap-3 rounded-md px-2 py-2 text-left transition-colors hover:bg-secondary/40"
                >
                  <span className="flex-1 truncate text-sm text-foreground">
                    {s.title || s.preview || '新对话'}
                  </span>
                  <span className="shrink-0 text-xs text-muted-foreground">
                    {formatRelative(s.updated_at)}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}

function formatRelative(iso: string): string {
  const diff = Date.now() - new Date(iso).getTime();
  const days = Math.floor(diff / 86_400_000);
  if (days === 0) return '今天';
  if (days === 1) return '昨天';
  if (days < 7) return `${days} 天前`;
  return new Date(iso).toLocaleDateString();
}
