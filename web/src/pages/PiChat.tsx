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

import type { DynamicToolUIPart, ReasoningUIPart, TextUIPart } from 'ai';
import { Sparkles } from 'lucide-react';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { buildWsUrl, type PublicWebEvent } from '@/adapters/rara-stream';
import { api } from '@/api/client';
import type { CascadeTrace, ExecutionTrace } from '@/api/kernel-types';
import type { ChatMessageData, ChatSession, ProviderInfo } from '@/api/types';
import {
  Conversation,
  ConversationContent,
  ConversationScrollButton,
} from '@/components/chat/ai-elements/conversation';
import { Message, MessageContent, MessageResponse } from '@/components/chat/ai-elements/message';
import {
  PromptInput,
  PromptInputBody,
  PromptInputFooter,
  PromptInputSubmit,
  PromptInputTextarea,
  PromptInputTools,
} from '@/components/chat/ai-elements/prompt-input';
import { Tool, ToolContent, ToolHeader } from '@/components/chat/ai-elements/tool';
import { CascadeModal } from '@/components/chat/CascadeModal';
import { ExecutionTraceModal } from '@/components/chat/ExecutionTraceModal';
import {
  applyRaraEvent,
  historyToUIMessages,
  type RaraUIMessage,
} from '@/components/chat/rara-to-uimessage';
import { ToolRenderer, toolHeaderSummary } from '@/components/chat/tool-renderers';
import { ChatSidebar } from '@/components/ChatSidebar';
import { RaraModelDialog } from '@/components/RaraModelDialog';
import { useSettingsModal } from '@/components/settings/SettingsModalProvider';
import { VoiceRecorder } from '@/components/VoiceRecorder';
import { readStoredSessionKey, writeStoredSessionKey } from '@/lib/active-session';

/**
 * Primary chat page mounted at `/`. Renders the ported ai-elements
 * `Conversation` + `Message` against rara's WebSocket chat stream, using the
 * `rara-to-uimessage` adapter as the bridge.
 */
export default function PiChat() {
  const { openSettings } = useSettingsModal();

  const [activeSession, setActiveSession] = useState<ChatSession | null>(null);
  const [messages, setMessages] = useState<RaraUIMessage[]>([]);
  const [composerText, setComposerText] = useState('');
  const [streaming, setStreaming] = useState(false);
  const [sidebarRefreshKey, setSidebarRefreshKey] = useState(0);
  const [modelDialogOpen, setModelDialogOpen] = useState(false);
  // Surfaces "Use rara's default" PATCH failures inside RaraModelDialog so the
  // user can retry without the dialog dismissing.
  const [resetError, setResetError] = useState<string | null>(null);

  // Cascade-trace modal state — fetched lazily when the user clicks the
  // "🔍 Cascade" trigger on a finalised assistant turn. Mirrors the PiChat
  // wiring (#1718): the kernel only assembles cascade entries via REST after
  // the turn completes, so the seq → trace fetch happens here, not in-stream.
  const [cascadeOpen, setCascadeOpen] = useState(false);
  const [cascadeTrace, setCascadeTrace] = useState<CascadeTrace | null>(null);
  const [cascadeLoading, setCascadeLoading] = useState(false);
  const [cascadeError, setCascadeError] = useState<string | null>(null);
  // Execution-trace modal state — opened from the "📊 详情" trigger and kept
  // distinct from the cascade viewer so the user can pick the lens per-click.
  const [execTraceOpen, setExecTraceOpen] = useState(false);
  const [execTrace, setExecTrace] = useState<ExecutionTrace | null>(null);
  const [execTraceLoading, setExecTraceLoading] = useState(false);
  const [execTraceError, setExecTraceError] = useState<string | null>(null);

  const wsRef = useRef<WebSocket | null>(null);
  // Tracks the session key the user most recently asked to load. A history
  // fetch that resolves after the user has switched away must NOT clobber the
  // newer session's state — we compare against this ref before applying.
  const activeSessionRef = useRef<string | null>(null);

  /** Switch to a session: load its message history and update state. */
  const selectSession = useCallback(async (session: ChatSession) => {
    // Tear down any in-flight stream from the previous session before we
    // start mutating state — otherwise its frames will land on the new
    // session via `setMessages` and bleed across sessions.
    wsRef.current?.close();
    wsRef.current = null;
    setStreaming(false);

    activeSessionRef.current = session.key;
    setActiveSession(session);
    writeStoredSessionKey(session.key);
    try {
      const rows = await api.get<ChatMessageData[]>(
        `/api/v1/chat/sessions/${encodeURIComponent(session.key)}/messages?limit=200`,
      );
      // Race guard: A→B→A switching can resolve B after A. If the user has
      // moved on, drop this response on the floor.
      if (activeSessionRef.current !== session.key) return;
      setMessages(historyToUIMessages(rows));
    } catch {
      if (activeSessionRef.current !== session.key) return;
      setMessages([]);
    }
  }, []);

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
        console.warn('PiChat: failed to load sessions', err);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [selectSession]);

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

  /** Reload the active session's history — used after VoiceRecorder finishes
   *  appending a transcribed user turn server-side, and after the WebSocket
   *  emits `done` so each finalised assistant turn picks up its persisted
   *  `seq` (via `RaraMessageMetadata`) for trace / cascade trigger fetches. */
  const reloadActiveMessages = useCallback(async () => {
    const key = activeSessionRef.current;
    if (!key) return;
    try {
      const rows = await api.get<ChatMessageData[]>(
        `/api/v1/chat/sessions/${encodeURIComponent(key)}/messages?limit=200`,
      );
      if (activeSessionRef.current !== key) return;
      setMessages(historyToUIMessages(rows));
    } catch (err) {
      console.warn('PiChat: failed to reload messages', err);
    }
  }, []);

  /** Send the composer text over a fresh WebSocket. */
  const sendMessage = useCallback(
    (rawText?: string) => {
      const text = (rawText ?? composerText).trim();
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
        console.error('PiChat: cannot build ws url', err);
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
        // The stream has no `seq` for in-flight assistant frames — rara only
        // assigns it when the turn lands in the kernel store. Refetch history
        // once the run finalises so the trace / cascade triggers can resolve
        // their per-turn seq from `RaraMessageMetadata`.
        if (event.type === 'done') {
          void reloadActiveMessages();
        }
      };

      ws.onerror = () => {
        setStreaming(false);
        // Surface transport-level failures (TLS handshake, auth reject, dropped
        // upgrade) to the user — without this they only see the spinner stop.
        setMessages((prev) =>
          applyRaraEvent(prev, { type: 'error', message: 'connection failed' }),
        );
      };

      ws.onclose = () => {
        setStreaming(false);
        if (wsRef.current === ws) wsRef.current = null;
      };
    },
    [activeSession, composerText, streaming, reloadActiveMessages],
  );

  /** Persist a provider/model pick from RaraModelDialog onto the active session. */
  const handleSelectProvider = useCallback(async (entry: ProviderInfo) => {
    const key = activeSessionRef.current;
    if (!key) {
      setModelDialogOpen(false);
      return;
    }
    try {
      await api.patch(`/api/v1/chat/sessions/${encodeURIComponent(key)}`, {
        model: entry.default_model,
        model_provider: entry.id,
      });
      if (activeSessionRef.current !== key) {
        setModelDialogOpen(false);
        return;
      }
      setActiveSession((prev) =>
        prev && prev.key === key
          ? { ...prev, model: entry.default_model, model_provider: entry.id }
          : prev,
      );
      setModelDialogOpen(false);
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      setResetError(`Failed to set model: ${msg}`);
    }
  }, []);

  /** Clear the per-session pin so `llm.default_provider` takes over. */
  const handleUseDefault = useCallback(async () => {
    const key = activeSessionRef.current;
    if (!key) {
      setModelDialogOpen(false);
      return;
    }
    setResetError(null);
    try {
      await api.patch(`/api/v1/chat/sessions/${encodeURIComponent(key)}`, {
        model: null,
        model_provider: null,
        thinking_level: null,
      });
      if (activeSessionRef.current !== key) {
        setModelDialogOpen(false);
        return;
      }
      setActiveSession((prev) =>
        prev && prev.key === key
          ? { ...prev, model: null, model_provider: null, thinking_level: null }
          : prev,
      );
      setModelDialogOpen(false);
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      setResetError(`Failed to reset model: ${msg}`);
    }
  }, []);

  // Drop any stale reset error when the dialog closes so it doesn't reappear
  // on next open.
  useEffect(() => {
    if (!modelDialogOpen) setResetError(null);
  }, [modelDialogOpen]);

  /** Open the cascade modal for a given turn's `seq`, fetching the trace
   *  lazily. A failed fetch surfaces inline inside the modal so the click is
   *  never silently swallowed. */
  const openCascade = useCallback((seq: number) => {
    const sessionKey = activeSessionRef.current;
    if (!sessionKey) return;
    setCascadeOpen(true);
    setCascadeTrace(null);
    setCascadeError(null);
    setCascadeLoading(true);
    api
      .get<CascadeTrace>(`/api/v1/chat/sessions/${encodeURIComponent(sessionKey)}/trace?seq=${seq}`)
      .then((trace) => {
        setCascadeTrace(trace);
      })
      .catch((e: unknown) => {
        const msg = e instanceof Error ? e.message : String(e);
        setCascadeError(msg);
      })
      .finally(() => {
        setCascadeLoading(false);
      });
  }, []);

  /** Open the per-turn execution-trace modal. A 404 surfaces as an inline
   *  error rather than silently closing — legacy turns recorded before trace
   *  persistence existed will land here. */
  const openExecTrace = useCallback((seq: number) => {
    const sessionKey = activeSessionRef.current;
    if (!sessionKey) return;
    setExecTraceOpen(true);
    setExecTrace(null);
    setExecTraceError(null);
    setExecTraceLoading(true);
    api
      .get<ExecutionTrace>(
        `/api/v1/chat/sessions/${encodeURIComponent(sessionKey)}/execution-trace?seq=${seq}`,
      )
      .then((trace) => {
        setExecTrace(trace);
      })
      .catch((e: unknown) => {
        const msg = e instanceof Error ? e.message : String(e);
        setExecTraceError(msg);
      })
      .finally(() => {
        setExecTraceLoading(false);
      });
  }, []);

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

  // Only the elevated thinking buckets get a header pill — `off`/`minimal`/
  // `low` are noise on the title row, and `null` means the session inherits
  // `llm.default_provider`'s level so the UI has nothing concrete to label.
  const thinkingPillLevel = useMemo(() => {
    const level = activeSession?.thinking_level;
    if (level === 'medium' || level === 'high' || level === 'xhigh') return level;
    return null;
  }, [activeSession?.thinking_level]);

  return (
    <div className="rara-chat flex h-screen w-screen">
      <ChatSidebar
        activeSessionKey={activeSession?.key}
        onSelect={(s) => void selectSession(s)}
        onNewSession={() => void newSession()}
        onOpenSearch={() => {
          /* TODO: wire search dialog */
        }}
        onOpenSettings={() => openSettings()}
        onDeleteSession={handleSessionDeleted}
        refreshKey={sidebarRefreshKey}
      />
      <main className="relative flex min-h-0 min-w-0 flex-1 flex-col">
        <header className="flex h-14 shrink-0 items-center justify-between gap-4 border-b border-border/60 px-6">
          <h1 className="flex min-w-0 flex-1 items-center gap-2 truncate text-sm font-semibold text-foreground">
            <span className="truncate">{headerTitle}</span>
            {thinkingPillLevel ? (
              <span className="bg-brand/10 text-brand ml-2 rounded-full px-2 py-0.5 text-[11px] font-medium">
                {thinkingPillLevel} thinking
              </span>
            ) : null}
          </h1>
        </header>

        <Conversation className="min-h-0 flex-1">
          <ConversationContent className="mx-auto w-full max-w-3xl">
            {messages.length === 0 ? (
              <ChatEmptyState
                disabled={!activeSession || streaming}
                onPick={(prompt) => {
                  // Drop the picked suggestion straight into the WebSocket
                  // sender — feels snappier than seeding the composer and
                  // making the user press enter, and the empty-state row
                  // never reappears once the first turn lands.
                  sendMessage(prompt);
                }}
              />
            ) : (
              messages.map((msg) => {
                const seq = msg.role === 'assistant' ? msg.metadata?.seq : undefined;
                return (
                  <Message key={msg.id} from={msg.role}>
                    <MessageContent>
                      {msg.parts.map((part, i) => (
                        <RenderPart
                          key={`${msg.id}-${i}`}
                          part={part as TextUIPart | ReasoningUIPart | DynamicToolUIPart}
                        />
                      ))}
                      {seq !== undefined ? (
                        <TraceTriggerRow
                          onOpenTrace={() => openExecTrace(seq)}
                          onOpenCascade={() => openCascade(seq)}
                        />
                      ) : null}
                    </MessageContent>
                  </Message>
                );
              })
            )}
          </ConversationContent>
          <ConversationScrollButton />
        </Conversation>

        <div className="mx-auto w-full max-w-3xl shrink-0 px-4 py-4">
          <PromptInput
            // PromptInput owns text in DOM, but we mirror it into `composerText`
            // so the submit button's disabled state and the WebSocket sender
            // can read the current value without poking the form ref.
            onSubmit={(message) => {
              sendMessage(message.text);
            }}
          >
            <PromptInputBody>
              <PromptInputTextarea
                value={composerText}
                onChange={(e) => setComposerText(e.currentTarget.value)}
                placeholder={activeSession ? 'Message rara…' : 'Select a session to start.'}
                disabled={!activeSession || streaming}
              />
            </PromptInputBody>
            <PromptInputFooter>
              <PromptInputTools>
                {/*
                  VoiceRecorder pushes the transcribed turn server-side, so
                  after it completes we refetch session history rather than
                  injecting a synthetic UIMessage. Mounted only when a session
                  exists — otherwise `getSessionKey` would return undefined
                  and the recorder couldn't post anywhere useful.
                */}
                {activeSession && (
                  <VoiceRecorder
                    className="!h-8 !w-8 !rounded-md !bg-transparent !shadow-none hover:!bg-accent"
                    getSessionKey={() => activeSessionRef.current ?? undefined}
                    onComplete={() => {
                      void reloadActiveMessages();
                    }}
                  />
                )}
              </PromptInputTools>
              <div className="flex items-center gap-1">
                <button
                  type="button"
                  onClick={() => setModelDialogOpen(true)}
                  disabled={!activeSession}
                  className="inline-flex items-center gap-1.5 rounded-full border border-border/60 px-2.5 py-1 font-mono text-xs text-muted-foreground transition-colors hover:border-border hover:text-foreground disabled:opacity-50"
                >
                  <Sparkles className="h-3 w-3" />
                  <span className="max-w-[160px] truncate">
                    {activeSession?.model_provider ?? 'auto'}
                  </span>
                </button>
                <PromptInputSubmit
                  {...(streaming ? { status: 'streaming' as const } : {})}
                  disabled={!activeSession || streaming || composerText.trim().length === 0}
                />
              </div>
            </PromptInputFooter>
          </PromptInput>
        </div>
      </main>
      <RaraModelDialog
        open={modelDialogOpen}
        onClose={() => setModelDialogOpen(false)}
        currentProvider={activeSession?.model_provider ?? null}
        onSelect={(entry) => {
          void handleSelectProvider(entry);
        }}
        onUseDefault={() => {
          void handleUseDefault();
        }}
        resetError={resetError}
      />
      <CascadeModal
        open={cascadeOpen}
        trace={cascadeTrace}
        loading={cascadeLoading}
        error={cascadeError}
        onClose={() => setCascadeOpen(false)}
      />
      <ExecutionTraceModal
        open={execTraceOpen}
        trace={execTrace}
        loading={execTraceLoading}
        error={execTraceError}
        onClose={() => setExecTraceOpen(false)}
      />
    </div>
  );
}

/** Suggested prompts shown on the empty state. Hardcoded for now — a future
 *  PR may pull these from kernel state (recent topics, skill registry). */
const SUGGESTED_PROMPTS: readonly string[] = [
  'What can rara do for me right now?',
  'Show me my recent sessions',
  "Explain rara's heartbeat architecture",
];

/** Empty-state hero rendered when the active session has no messages.
 *  Centred lockup + tagline + 3 suggestion cards that submit straight to the
 *  WebSocket on click. `disabled` mirrors the composer's disabled state so a
 *  user can't fire a suggestion mid-stream or before a session exists. */
function ChatEmptyState({
  disabled,
  onPick,
}: {
  disabled: boolean;
  onPick: (prompt: string) => void;
}) {
  return (
    <div className="flex flex-col items-center justify-center gap-6 px-4 py-16">
      <div className="flex flex-col items-center gap-2">
        <div className="text-[32px] font-semibold tracking-tight text-foreground">rara</div>
        <div className="text-sm text-muted-foreground">How can I help today?</div>
      </div>
      <div className="grid w-full grid-cols-1 gap-2 sm:grid-cols-3">
        {SUGGESTED_PROMPTS.map((prompt) => (
          <button
            key={prompt}
            type="button"
            disabled={disabled}
            onClick={() => onPick(prompt)}
            className="rounded-xl border border-border/60 p-4 text-left text-sm text-foreground transition-colors hover:bg-muted/40 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50"
          >
            {prompt}
          </button>
        ))}
      </div>
    </div>
  );
}

/** Inline trigger row rendered below each finalised assistant turn. Mirrors
 *  PiChat's "📊 详情" / "🔍 Cascade" buttons (#1718) but rendered as plain
 *  React rather than via the Lit message-renderer hijack. Only mounted when
 *  the turn has a persisted `seq` — otherwise the trace endpoints would 404
 *  and the buttons would mislead the user. */
function TraceTriggerRow({
  onOpenTrace,
  onOpenCascade,
}: {
  onOpenTrace: () => void;
  onOpenCascade: () => void;
}) {
  return (
    <div className="mt-1 flex gap-1.5 text-xs">
      <button
        type="button"
        onClick={onOpenTrace}
        className="inline-flex items-center gap-1 rounded-md px-2 py-0.5 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
        title="详情"
      >
        <span aria-hidden>📊</span>
        <span>详情</span>
      </button>
      <button
        type="button"
        onClick={onOpenCascade}
        className="inline-flex items-center gap-1 rounded-md px-2 py-0.5 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
        title="Cascade"
      >
        <span aria-hidden>🔍</span>
        <span>Cascade</span>
      </button>
    </div>
  );
}

/** Render one UIMessage part. Text/reasoning go through Streamdown so
 *  markdown (bold, headings, code, tables) renders correctly even mid-stream;
 *  tool calls dispatch to a per-tool rich renderer. */
function RenderPart({ part }: { part: TextUIPart | ReasoningUIPart | DynamicToolUIPart }) {
  if (part.type === 'text') {
    return (
      <div className="prose prose-sm dark:prose-invert max-w-none text-sm text-foreground">
        <MessageResponse>{part.text}</MessageResponse>
      </div>
    );
  }
  if (part.type === 'reasoning') {
    return (
      <div className="rounded-md border border-border/40 bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
        <div className="mb-1 font-medium uppercase tracking-wide">Reasoning</div>
        <MessageResponse>{part.text}</MessageResponse>
      </div>
    );
  }
  // dynamic-tool — render via the ported Tool card with a per-tool body.
  const summary = toolHeaderSummary(part);
  const headerProps = {
    type: 'dynamic-tool' as const,
    toolName: part.toolName,
    state: part.state,
    className: '[&_span]:truncate',
    ...(summary ? { title: `${part.toolName} · ${summary}` } : {}),
  };
  return (
    <Tool>
      <ToolHeader {...headerProps} />
      <ToolContent>
        <ToolRenderer part={part} />
      </ToolContent>
    </Tool>
  );
}
