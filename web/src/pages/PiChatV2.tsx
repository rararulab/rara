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

import type { DynamicToolUIPart, ReasoningUIPart, TextUIPart, UIMessage } from 'ai';
import { Send } from 'lucide-react';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { buildWsUrl, type PublicWebEvent } from '@/adapters/rara-stream';
import { api } from '@/api/client';
import type { ChatMessageData, ChatSession } from '@/api/types';
import {
  Conversation,
  ConversationContent,
  ConversationEmptyState,
  ConversationScrollButton,
} from '@/components/chat/ai-elements/conversation';
import { Message, MessageContent } from '@/components/chat/ai-elements/message';
import {
  Tool,
  ToolContent,
  ToolHeader,
  ToolInput,
  ToolOutput,
} from '@/components/chat/ai-elements/tool';
import { applyRaraEvent, historyToUIMessages } from '@/components/chat/rara-to-uimessage';
import { ChatSidebar } from '@/components/ChatSidebar';
import { useSettingsModal } from '@/components/settings/SettingsModalProvider';
import { Button } from '@/components/ui/button';

const ACTIVE_SESSION_KEY = 'rara.activeSessionKey';

function readStoredSessionKey(): string | null {
  try {
    return localStorage.getItem(ACTIVE_SESSION_KEY);
  } catch {
    return null;
  }
}

function writeStoredSessionKey(key: string): void {
  try {
    localStorage.setItem(ACTIVE_SESSION_KEY, key);
  } catch {
    /* ignore */
  }
}

/**
 * Parallel chat page mounted at `/chat-v2`. Renders the ported ai-elements
 * `Conversation` + `Message` against rara's WebSocket chat stream, using the
 * `rara-to-uimessage` adapter as the bridge.
 *
 * This page intentionally duplicates a thin slice of `PiChat.tsx` (sidebar
 * wiring, session list, history fetch). PR7 deletes the original PiChat once
 * the new shell reaches feature parity, at which point the duplication
 * collapses.
 */
export default function PiChatV2() {
  const { openSettings } = useSettingsModal();

  const [activeSession, setActiveSession] = useState<ChatSession | null>(null);
  const [messages, setMessages] = useState<UIMessage[]>([]);
  const [composerText, setComposerText] = useState('');
  const [streaming, setStreaming] = useState(false);
  const [sidebarRefreshKey, setSidebarRefreshKey] = useState(0);

  const wsRef = useRef<WebSocket | null>(null);

  // Initial session bootstrap: pick the stored key if it still exists,
  // otherwise the most recent.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const list = await api.get<ChatSession[]>('/api/v1/chat/sessions?limit=50');
        if (cancelled) return;
        const stored = readStoredSessionKey();
        const initial = list.find((s) => s.key === stored) ?? list[0] ?? null;
        if (initial) await selectSession(initial);
      } catch (err) {
        console.warn('PiChatV2: failed to load sessions', err);
      }
    })();
    return () => {
      cancelled = true;
    };
    // selectSession is stable via useCallback below
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /** Switch to a session: load its message history and update state. */
  const selectSession = useCallback(async (session: ChatSession) => {
    setActiveSession(session);
    writeStoredSessionKey(session.key);
    try {
      const rows = await api.get<ChatMessageData[]>(
        `/api/v1/chat/sessions/${encodeURIComponent(session.key)}/messages?limit=200`,
      );
      setMessages(historyToUIMessages(rows));
    } catch {
      setMessages([]);
    }
  }, []);

  /** Create a fresh session and switch to it. */
  const newSession = useCallback(async () => {
    const created = await api.post<ChatSession>('/api/v1/chat/sessions', {});
    setSidebarRefreshKey((k) => k + 1);
    await selectSession(created);
  }, [selectSession]);

  /** After a delete, fall back to the most recent remaining session. */
  const handleSessionDeleted = useCallback(
    (key: string, fallback: ChatSession | null) => {
      setSidebarRefreshKey((k) => k + 1);
      if (activeSession?.key !== key) return;
      if (fallback) void selectSession(fallback);
      else void newSession();
    },
    [activeSession?.key, newSession, selectSession],
  );

  /** Send the composer text over a fresh WebSocket. */
  const sendMessage = useCallback(() => {
    const text = composerText.trim();
    if (!text || !activeSession || streaming) return;
    setComposerText('');

    // Optimistically push the user message so the UI updates immediately.
    setMessages((prev) => [
      ...prev,
      {
        id: `user-${Date.now()}`,
        role: 'user',
        parts: [{ type: 'text', text, state: 'done' } satisfies TextUIPart],
      },
    ]);
    setStreaming(true);

    let wsUrl: string;
    try {
      wsUrl = buildWsUrl(activeSession.key);
    } catch (err) {
      setStreaming(false);
      console.error('PiChatV2: cannot build ws url', err);
      return;
    }

    const ws = new WebSocket(wsUrl);
    wsRef.current = ws;

    ws.onopen = () => {
      ws.send(text);
    };

    ws.onmessage = (ev: MessageEvent) => {
      let event: PublicWebEvent;
      try {
        event = JSON.parse(ev.data as string) as PublicWebEvent;
      } catch {
        return;
      }
      setMessages((prev) => applyRaraEvent(prev, event));
    };

    ws.onerror = () => {
      setStreaming(false);
    };

    ws.onclose = () => {
      setStreaming(false);
      if (wsRef.current === ws) wsRef.current = null;
    };
  }, [activeSession, composerText, streaming]);

  // Cleanly close the socket when the page unmounts.
  useEffect(() => {
    return () => {
      wsRef.current?.close();
      wsRef.current = null;
    };
  }, []);

  const headerTitle = useMemo(() => {
    if (!activeSession) return '新对话';
    return activeSession.title || activeSession.preview || '新对话';
  }, [activeSession]);

  return (
    <div className="flex h-screen w-screen">
      <ChatSidebar
        activeSessionKey={activeSession?.key}
        onSelect={(s) => void selectSession(s)}
        onNewSession={() => void newSession()}
        onOpenSearch={() => {
          /* TODO(PR6): wire search dialog */
        }}
        onOpenSettings={() => openSettings()}
        onDeleteSession={handleSessionDeleted}
        refreshKey={sidebarRefreshKey}
      />
      <main className="relative flex min-h-0 min-w-0 flex-1 flex-col">
        <header className="flex h-14 shrink-0 items-center justify-between gap-4 border-b border-border/60 px-6">
          <h1 className="min-w-0 flex-1 truncate text-sm font-semibold text-foreground">
            {headerTitle}
          </h1>
          {activeSession?.model && (
            <span className="shrink-0 truncate rounded-full border border-border/60 px-2.5 py-0.5 text-xs text-muted-foreground">
              {activeSession.model}
            </span>
          )}
          <span className="shrink-0 truncate rounded-full border border-amber-500/40 bg-amber-500/10 px-2.5 py-0.5 text-xs text-amber-700 dark:text-amber-300">
            chat-v2 preview
          </span>
        </header>

        <Conversation className="min-h-0 flex-1">
          <ConversationContent className="mx-auto w-full max-w-3xl">
            {messages.length === 0 ? (
              <ConversationEmptyState
                title="No messages yet"
                description="Type below to start a conversation."
              />
            ) : (
              messages.map((msg) => (
                <Message key={msg.id} from={msg.role}>
                  <MessageContent>
                    {msg.parts.map((part, i) => (
                      <RenderPart
                        key={`${msg.id}-${i}`}
                        part={part as TextUIPart | ReasoningUIPart | DynamicToolUIPart}
                      />
                    ))}
                  </MessageContent>
                </Message>
              ))
            )}
          </ConversationContent>
          <ConversationScrollButton />
        </Conversation>

        <form
          className="mx-auto w-full max-w-3xl shrink-0 px-4 py-4"
          onSubmit={(e) => {
            e.preventDefault();
            sendMessage();
          }}
        >
          <div className="flex items-end gap-2 rounded-lg border border-border bg-background p-2 focus-within:border-ring">
            <textarea
              value={composerText}
              onChange={(e) => setComposerText(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && !e.shiftKey) {
                  e.preventDefault();
                  sendMessage();
                }
              }}
              rows={2}
              placeholder={activeSession ? 'Message rara…' : 'Select a session to start.'}
              disabled={!activeSession || streaming}
              className="min-h-[2.5rem] flex-1 resize-none border-0 bg-transparent text-sm text-foreground outline-none placeholder:text-muted-foreground"
            />
            <Button
              type="submit"
              size="icon"
              disabled={!activeSession || streaming || composerText.trim().length === 0}
            >
              <Send className="size-4" />
              <span className="sr-only">Send</span>
            </Button>
          </div>
        </form>
      </main>
    </div>
  );
}

/** Render one UIMessage part. PR3 will replace tool-card semantics with the
 *  per-tool rich renderer; this is the plaintext baseline. */
function RenderPart({ part }: { part: TextUIPart | ReasoningUIPart | DynamicToolUIPart }) {
  if (part.type === 'text') {
    return <div className="whitespace-pre-wrap text-sm text-foreground">{part.text}</div>;
  }
  if (part.type === 'reasoning') {
    return (
      <div className="whitespace-pre-wrap rounded-md border border-border/40 bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
        <div className="mb-1 font-medium uppercase tracking-wide">Reasoning</div>
        {part.text}
      </div>
    );
  }
  // dynamic-tool — render via the ported Tool card.
  const errorText = part.state === 'output-error' ? part.errorText : undefined;
  const output = part.state === 'output-available' ? part.output : undefined;
  return (
    <Tool>
      <ToolHeader type="dynamic-tool" toolName={part.toolName} state={part.state} />
      <ToolContent>
        <ToolInput input={part.input} />
        <ToolOutput output={output} errorText={errorText} />
      </ToolContent>
    </Tool>
  );
}
