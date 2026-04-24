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

import { Agent } from '@mariozechner/pi-agent-core';
import {
  AppStorage,
  setAppStorage,
  SessionsStore,
  SettingsStore,
  // Stub stores only — rara's admin settings modal is the real source of
  // truth for provider keys and custom providers (see #1581).
  ProviderKeysStore,
  CustomProvidersStore,
  defaultConvertToLlm,
  registerMessageRenderer,
  // Importing the extract-document tool triggers a module-level
  // `registerToolRenderer("extract_document", ...)` side effect so
  // pi-mono can render server-triggered document-extraction tool calls.
  extractDocumentTool,
} from '@mariozechner/pi-web-ui';
import { html } from 'lit';
import { useEffect, useRef, useCallback, useState } from 'react';

// Reference the tool so Vite's tree-shaker keeps the module (and its
// `registerToolRenderer` side effect) in the bundle. The actual tool
// object is executed server-side; the renderer is what matters here.
void extractDocumentTool;

import {
  assistantSeqByRef,
  messagesForArtifactReconstruction,
  toAgentMessages,
  toolResultByCallId,
} from './pi-chat-messages';

import { RaraStorageBackend } from '@/adapters/rara-storage';
import { createRaraStreamFn } from '@/adapters/rara-stream';
import { api, settingsApi } from '@/api/client';
import type { CascadeTrace, ExecutionTrace } from '@/api/kernel-types';
import type { ProviderInfo } from '@/api/types';
import type { ChatSession, ChatMessageData, ThinkingLevel } from '@/api/types';
import { AgentLiveCard } from '@/components/agent-live/AgentLiveCard';
import { liveRunStore } from '@/components/agent-live/live-run-store';
import { AlmaCaret } from '@/components/AlmaCaret';
import { CascadeModal } from '@/components/chat/CascadeModal';
import { ExecutionTraceModal } from '@/components/chat/ExecutionTraceModal';
import { ChatSidebar } from '@/components/ChatSidebar';
import { RaraModelDialog } from '@/components/RaraModelDialog';
import { SessionSearchDialog } from '@/components/SessionSearchDialog';
import { useSettingsModal } from '@/components/settings/SettingsModalProvider';
import { VoiceRecorder } from '@/components/VoiceRecorder';
import { useLiveCardHeight } from '@/hooks/use-live-card-height';
import { useSessionDelete } from '@/hooks/use-session-delete';
import { UNKNOWN_MODEL_SENTINEL, isUnknownModel, syntheticModel } from '@/lib/synthetic-model';
import { registerRaraToolRenderers } from '@/tools/rara-tool-renderers';

const ACTIVE_SESSION_KEY = 'rara.activeSessionKey';

function readStoredSessionKey(): string | null {
  try {
    return localStorage.getItem(ACTIVE_SESSION_KEY);
  } catch {
    return null;
  }
}

function writeStoredSessionKey(key: string | null): void {
  try {
    if (key) localStorage.setItem(ACTIVE_SESSION_KEY, key);
    else localStorage.removeItem(ACTIVE_SESSION_KEY);
  } catch {
    /* ignore */
  }
}

/**
 * True when the given provider id is still present in rara's routable
 * catalog (from `/api/v1/chat/providers`). Fails open when the catalog
 * has not been loaded yet so session restore isn't blocked waiting on
 * an unrelated fetch — a stale provider caught later on send is still
 * cheaper than blocking the whole chat init.
 */
function isRoutableProvider(
  catalog: Set<string> | null,
  provider: string | null | undefined,
): boolean {
  if (!provider) return false;
  if (!catalog) return true;
  return catalog.has(provider);
}

/**
 * Look up the admin-configured default `(provider, model)` pair in the
 * rara settings store. Returns `null` if the admin has not paired a
 * default model with their default provider — the caller falls back to
 * the unknown sentinel and pi-web-ui's composer pill goes blank instead
 * of inventing a model from its own hard-coded catalog (which would
 * surface a ghost "gemini-2.5-flash-lite" on a minimax-default install).
 */
async function resolveAdminDefaultModel(): Promise<{
  provider: string;
  model: string;
} | null> {
  try {
    const settings = await settingsApi.list();
    const provider = settings['llm.default_provider']?.trim();
    if (!provider) return null;
    const model = settings[`llm.providers.${provider}.default_model`]?.trim();
    if (!model) {
      console.warn(
        `Admin default provider \`${provider}\` has no default_model set — composer pill will show unknown.`,
      );
      return null;
    }
    return { provider, model };
  } catch (e: unknown) {
    console.warn('Failed to resolve admin default provider:', e);
    return null;
  }
}

/**
 * The rara backend accepts the same six buckets pi-mono exposes
 * (`off | minimal | low | medium | high | xhigh`), so the chat-panel
 * selector round-trips verbatim. This guard just narrows the type.
 */
function asThinkingLevel(level: string | undefined): ThinkingLevel | null {
  switch (level) {
    case 'off':
    case 'minimal':
    case 'low':
    case 'medium':
    case 'high':
    case 'xhigh':
      return level;
    default:
      return null;
  }
}

/**
 * DOM events dispatched by the Lit assistant-message renderer when the
 * user clicks one of the per-turn detail buttons. Both carry the same
 * `seq` payload (resolved via {@link assistantSeqByRef}) and are
 * handled by parallel React effects below.
 *
 * Two separate events rather than one discriminated payload so each
 * handler can own its own modal state without switching on a tag.
 */
const CASCADE_TRACE_EVENT = 'rara:cascade-trace';
const EXECUTION_TRACE_EVENT = 'rara:execution-trace';

interface TraceEventDetail {
  seq: number;
}

/**
 * Register a Lit message renderer that wraps pi-web-ui's built-in
 * `<assistant-message>` element and appends two trace-detail buttons:
 *
 * - "📊 详情" → dispatches {@link EXECUTION_TRACE_EVENT}, opening a
 *   high-level per-turn summary (rationale / thinking / plan / tools /
 *   usage) matching Telegram's "📊 详情" button.
 * - "🔍 Cascade" → dispatches {@link CASCADE_TRACE_EVENT}, opening the
 *   tick-level tape replay (kept for debugging the agent loop; mirrors
 *   Telegram's "🔍 Cascade" button).
 *
 * Both dispatch a bubbling CustomEvent carrying the persisted `seq`
 * resolved via {@link assistantSeqByRef}; the React layer below owns
 * the two modals separately.
 *
 * The renderer must rebuild the same `toolResultsById` lookup that
 * `MessageList` normally hands `<assistant-message>` — otherwise paired
 * tool results would not render under the call. The `agentResolver`
 * closure gives us that map at click time without re-registering on
 * every message-list change.
 *
 * Alignment note: the button row uses `pl-[2.75rem]` to match the
 * assistant-message bubble's left padding (set in `index.css` to make
 * room for rara's avatar). Without this the buttons would stick to the
 * container's left edge and visually detach from the bubble above.
 *
 * Skips placeholder turns with no mapped seq (e.g. mid-stream assistant
 * frames not yet persisted) — there's no row to ask the trace endpoint
 * for, and showing a button that 404s would be misleading.
 *
 * Idempotent: calling this multiple times leaves only the latest
 * registration in pi-web-ui's renderer map (a Map.set overwrite), which
 * is what we want during HMR.
 */
function registerCascadeAssistantRenderer(agentResolver: () => Agent | null): void {
  registerMessageRenderer('assistant', {
    render(message) {
      const seq = assistantSeqByRef.get(message);
      const showButtons = seq !== undefined;
      // Rebuild the toolResult lookup. Historical messages live in
      // `toolResultByCallId` because `toAgentMessages` no longer emits
      // standalone tool-result bubbles (#1718). Live streaming frames
      // still land in `agent.state.messages` as `toolResult` entries
      // (pi-agent-core's post-stream loop pushes them after the relay
      // tool resolves), so we merge both sources here — streaming
      // wins on key collision so a fresher result from the current
      // turn can override a stale persisted one.
      const agent = agentResolver();
      const resultByCallId = new Map<string, import('@mariozechner/pi-ai').ToolResultMessage>(
        toolResultByCallId,
      );
      if (agent) {
        for (const m of agent.state.messages) {
          if (m.role === 'toolResult') {
            const tr = m as import('@mariozechner/pi-ai').ToolResultMessage;
            resultByCallId.set(tr.toolCallId, tr);
          }
        }
      }
      const dispatchTrace = (eventName: string) => (e: Event) => {
        e.stopPropagation();
        if (seq === undefined) return;
        const detail: TraceEventDetail = { seq };
        document.dispatchEvent(
          new CustomEvent<TraceEventDetail>(eventName, {
            detail,
            bubbles: true,
          }),
        );
      };
      return html`
        <div class="rara-assistant-with-trace">
          <assistant-message
            .message=${message}
            .tools=${agent?.state.tools ?? []}
            .isStreaming=${false}
            .toolResultsById=${resultByCallId}
            .hideToolCalls=${false}
          ></assistant-message>
          ${showButtons
            ? html`
                <div class="mt-1 flex justify-start gap-1 pl-[2.75rem]">
                  <button
                    type="button"
                    class="rara-trace-trigger inline-flex items-center gap-1 rounded-md px-2 py-0.5 text-xs text-muted-foreground transition hover:bg-accent hover:text-foreground"
                    title="查看本轮执行摘要（rationale / thinking / plan / tools / usage）"
                    @click=${dispatchTrace(EXECUTION_TRACE_EVENT)}
                  >
                    <span aria-hidden>📊</span>
                    <span>详情</span>
                  </button>
                  <button
                    type="button"
                    class="rara-cascade-trigger inline-flex items-center gap-1 rounded-md px-2 py-0.5 text-xs text-muted-foreground transition hover:bg-accent hover:text-foreground"
                    title="查看本轮 cascade 执行详情（tick-level tape replay）"
                    @click=${dispatchTrace(CASCADE_TRACE_EVENT)}
                  >
                    <span aria-hidden>🔍</span>
                    <span>Cascade</span>
                  </button>
                </div>
              `
            : null}
        </div>
      `;
    },
  });
}

// Session list now lives in `ChatSidebar`; legacy slide-over deleted
// during the persistent-sidebar refactor — see #1585.
/**
 * Fullscreen wrapper that mounts pi-web-ui's <pi-chat-panel> Web Component,
 * wiring it up to rara's storage backend and WebSocket stream function.
 */
export default function PiChat() {
  const containerRef = useRef<HTMLDivElement>(null);
  // Live-card scroll-padding wiring: `liveCardEl` measures the rendered
  // card; `mainEl` receives the `--rara-live-card-h` CSS variable that
  // scopes the padding to this chat surface. Both are tracked via
  // `useState` + callback refs (rather than `useRef`) because the
  // wrapper div mounts conditionally on `!isInitializing`; effect
  // dependencies on `useRef` objects do not re-fire when `.current`
  // mutates, so a ref-based wiring missed the late-mounting wrapper
  // entirely and the CSS variable was never written.
  const [liveCardEl, setLiveCardEl] = useState<HTMLDivElement | null>(null);
  const [mainEl, setMainEl] = useState<HTMLElement | null>(null);
  const initRef = useRef(false);
  const agentRef = useRef<Agent | null>(null);
  const chatPanelRef = useRef<import('@mariozechner/pi-web-ui').ChatPanel | null>(null);
  // Tracks the last successfully-persisted (model, provider, thinking)
  // triple so onBeforeSend can skip no-op PATCHes on every send.
  const lastPersistedRef = useRef<{
    model: string | null;
    provider: string | null;
    thinking: string | null;
  } | null>(null);
  // Snapshot of rara-side provider ids currently routable by the kernel.
  // Used to reject stale `model_provider` values persisted before the
  // provider catalog shrank (e.g. leftover pi-mono `google` selections
  // from the pre-#1554 selector). `null` = not yet loaded; we fail-open
  // in that window so the restore isn't blocked on an unrelated fetch.
  const validProvidersRef = useRef<Set<string> | null>(null);
  // Guards against double-invocations of `handleUseDefault` while a PATCH
  // + settings fetch is still in flight. Backend no-ops the duplicate
  // write (see #1569 round-1 fix) but the UI would still redundantly
  // refetch settings and reset the composer state.
  const resetInflight = useRef(false);
  const [isInitializing, setIsInitializing] = useState(true);
  const [sidebarRefreshKey, setSidebarRefreshKey] = useState(0);
  // Active session metadata — surfaced in the main-area header so the
  // current chat's title sits above its messages (kimi-style). Updated
  // from switchSession / newSession / initial mount; a refetch fires
  // after the first send so backend-assigned titles appear promptly.
  const [activeSession, setActiveSession] = useState<ChatSession | null>(null);
  // Bump to force ChatSidebar to refetch the session list (e.g. after
  // creating a new session or sending the first message of a fresh one).
  const [modelDialogOpen, setModelDialogOpen] = useState(false);
  const [resetError, setResetError] = useState<string | null>(null);
  // `true` when the active session has no messages — we render a welcome
  // overlay in that window so the chat page isn't just an input box on
  // empty canvas. Flipped off on the first send and on session switches
  // that land on a populated session.
  const [showWelcome, setShowWelcome] = useState(true);
  const { openSettings } = useSettingsModal();
  // Cascade trace viewer state — opened when the user clicks the "📊 详情"
  // button injected into each assistant message by the custom Lit renderer
  // registered below. The seq → trace fetch is lazy: the kernel does not
  // stream cascade data, the UI only assembles it via REST after a turn
  // finishes (see `service.get_cascade_trace`).
  const [cascadeOpen, setCascadeOpen] = useState(false);
  const [cascadeTrace, setCascadeTrace] = useState<CascadeTrace | null>(null);
  const [cascadeLoading, setCascadeLoading] = useState(false);
  const [cascadeError, setCascadeError] = useState<string | null>(null);
  // Execution-trace modal state — populated from the new "📊 详情"
  // button (kept distinct from the cascade viewer so the user can pick
  // the right lens per-click).
  const [execTraceOpen, setExecTraceOpen] = useState(false);
  const [execTrace, setExecTrace] = useState<ExecutionTrace | null>(null);
  const [execTraceLoading, setExecTraceLoading] = useState(false);
  const [execTraceError, setExecTraceError] = useState<string | null>(null);
  // Cmd+K session-search palette.
  const [searchOpen, setSearchOpen] = useState(false);
  const [recentSessions, setRecentSessions] = useState<ChatSession[]>([]);

  // Global Cmd+K / Ctrl+K shortcut — toggles the search palette. We
  // register directly on `window` (instead of a wrapper hook) because
  // the dialog is the only consumer and there is no other keyboard
  // shortcut story in this file yet.
  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') {
        event.preventDefault();
        setSearchOpen((prev) => !prev);
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, []);

  // Refetch recent sessions whenever the palette opens so the empty-query
  // list reflects freshly-created or renamed sessions. Cheap (one
  // request) and keeps the palette in sync with the sidebar without
  // piping its state down.
  useEffect(() => {
    if (!searchOpen) return;
    let alive = true;
    api
      .get<ChatSession[]>('/api/v1/chat/sessions?limit=20&offset=0')
      .then((list) => {
        if (alive) setRecentSessions(list);
      })
      .catch(() => {
        if (alive) setRecentSessions([]);
      });
    return () => {
      alive = false;
    };
  }, [searchOpen, sidebarRefreshKey]);

  // Clear any stale reset-error banner whenever the model dialog is
  // closed — regardless of close path (backdrop click, successful
  // select, successful reset). Co-locating the clear here prevents the
  // banner from leaking into the next dialog opening.
  useEffect(() => {
    if (!modelDialogOpen) setResetError(null);
  }, [modelDialogOpen]);

  // Bridge between the Lit assistant-message renderer and React: when
  // the user clicks a "📊 详情" button, the renderer dispatches a
  // bubbling CustomEvent on `document` carrying the persisted `seq`
  // (resolved via `assistantSeqByRef`). A failed/empty fetch shows an
  // inline state in the modal rather than swallowing the click silently.
  useEffect(() => {
    const handler = (ev: Event) => {
      const ce = ev as CustomEvent<TraceEventDetail>;
      const seq = ce.detail?.seq;
      const sessionKey = agentRef.current?.sessionId;
      if (seq === undefined || !sessionKey) return;
      setCascadeOpen(true);
      setCascadeTrace(null);
      setCascadeError(null);
      setCascadeLoading(true);
      api
        .get<CascadeTrace>(
          `/api/v1/chat/sessions/${encodeURIComponent(sessionKey)}/trace?seq=${seq}`,
        )
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
    };
    document.addEventListener(CASCADE_TRACE_EVENT, handler);
    return () => document.removeEventListener(CASCADE_TRACE_EVENT, handler);
  }, []);

  // Bridge for the "📊 详情" button — fetches the persisted
  // `ExecutionTrace` for the clicked turn. A 404 is surfaced as an
  // inline error row rather than silently closing the modal so the
  // user understands why the view is empty (e.g. legacy turn recorded
  // before trace persistence existed).
  useEffect(() => {
    const handler = (ev: Event) => {
      const ce = ev as CustomEvent<TraceEventDetail>;
      const seq = ce.detail?.seq;
      const sessionKey = agentRef.current?.sessionId;
      if (seq === undefined || !sessionKey) return;
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
    };
    document.addEventListener(EXECUTION_TRACE_EVENT, handler);
    return () => document.removeEventListener(EXECUTION_TRACE_EVENT, handler);
  }, []);

  /** Switch the agent to a different session, loading its history. */
  const switchSession = useCallback(async (session: ChatSession) => {
    const agent = agentRef.current;
    if (!agent) return;
    agent.clearMessages();
    // Drop the agent-live card's in-memory runs for the previous session
    // so the sticky card doesn't leak across session switches.
    if (agent.sessionId && agent.sessionId !== session.key) {
      liveRunStore.reset(agent.sessionId);
    }
    agent.sessionId = session.key;
    setActiveSession(session);
    writeStoredSessionKey(session.key);
    // Optimistically hide the welcome overlay during the switch: the
    // backend's `message_count` is unreliable (always 0 in the listing
    // for older sessions, see #1585 round-2 notes), so trusting it
    // here would flash the RARA overlay on every click before the
    // messages arrive. We flip it back on after `list_messages` if
    // the session really is empty.
    setShowWelcome(false);

    // Restore the session's persisted model + thinking-level so the
    // model pill in the composer reflects the last settings used for
    // this conversation. We build a synthetic pi-ai Model rather than
    // looking the pair up in pi-ai's catalog — rara's provider ids
    // (`kimi`, `openrouter`, `scnet`, ...) do not exist there.
    //
    // Guard: reject restored providers that are no longer in the rara
    // routable catalog. Stale records from older builds (e.g. pi-mono's
    // `google`/`anthropic`) would otherwise paint a ghost selection
    // into the composer pill.
    if (
      session.model &&
      session.model_provider &&
      isRoutableProvider(validProvidersRef.current, session.model_provider)
    ) {
      agent.state.model = syntheticModel(session.model_provider, session.model);
    } else {
      // Unpinned session — seed the composer pill with rara's admin
      // default so it reads "minimax: MiniMax-M2.7" rather than
      // pi-web-ui's hard-coded catalog fallback (`gemini-2.5-*`).
      const resolved = await resolveAdminDefaultModel();
      if (resolved && agentRef.current?.sessionId === session.key) {
        agent.state.model = syntheticModel(resolved.provider, resolved.model);
      }
    }
    if (session.thinking_level) {
      agent.state.thinkingLevel = session.thinking_level;
    }
    // Reset the dedup ref to match the session that was just loaded so
    // onBeforeSend correctly re-PATCHes if the user changes selection
    // away from the restored values, and skips the identity write
    // otherwise.
    lastPersistedRef.current = {
      model: session.model ?? null,
      provider: session.model_provider ?? null,
      thinking: session.thinking_level ?? null,
    };

    try {
      const msgs = await api.get<ChatMessageData[]>(
        `/api/v1/chat/sessions/${encodeURIComponent(session.key)}/messages?limit=200`,
      );
      const agentMsgs = toAgentMessages(msgs);
      if (agentMsgs.length > 0) {
        agent.replaceMessages(agentMsgs);
      } else if (agentRef.current?.sessionId === session.key) {
        // Really an empty session (not just a stale backend count) —
        // reveal the welcome overlay now that we know for sure.
        setShowWelcome(true);
      }
      // Rebuild the artifacts panel from the same message list so switching
      // back to a session restores every previously-created artifact.
      await chatPanelRef.current?.artifactsPanel?.reconstructFromMessages(
        messagesForArtifactReconstruction(agentMsgs),
      );
    } catch {
      /* session may have no messages yet */
    }
    // Always trigger re-render after switching — even for empty sessions
    // so cleared messages are reflected in the UI.
    chatPanelRef.current?.agentInterface?.requestUpdate();
    // Drop focus into the composer so the user can start typing
    // immediately without a mouse click. Pi-web-ui's textarea mounts
    // lazily so we defer to the next frame and, for added belt, again
    // after the lit element completes its update pass.
    requestAnimationFrame(() => {
      const ta = document.querySelector<HTMLTextAreaElement>('textarea');
      ta?.focus();
    });
  }, []);

  /** Reload current session messages (e.g. after voice message completes). */
  const reloadMessages = useCallback(async () => {
    const agent = agentRef.current;
    if (!agent?.sessionId) return;
    try {
      const msgs = await api.get<ChatMessageData[]>(
        `/api/v1/chat/sessions/${encodeURIComponent(agent.sessionId)}/messages?limit=200`,
      );
      const agentMsgs = toAgentMessages(msgs);
      agent.replaceMessages(agentMsgs);
      await chatPanelRef.current?.artifactsPanel?.reconstructFromMessages(
        messagesForArtifactReconstruction(agentMsgs),
      );
      chatPanelRef.current?.agentInterface?.requestUpdate();
    } catch {
      /* ignore */
    }
  }, []);

  /** Create a new empty session and switch to it. */
  const newSession = useCallback(async () => {
    const created = await api.post<ChatSession>('/api/v1/chat/sessions', {});
    void switchSession(created);
    setSidebarRefreshKey((k) => k + 1);
  }, [switchSession]);

  /**
   * Handle session deletion from the sidebar. The decision + dispatch
   * wiring lives in `useSessionDelete` so both the pure decision and
   * the switch/create-new side effects are covered by unit tests.
   */
  const handleSessionDeleted = useSessionDelete<ChatSession>({
    activeSessionKey: activeSession?.key,
    switchSession,
    newSession,
  });

  /**
   * Reset the session's pinned (model, provider, thinking) triple so the
   * backend falls back to `llm.default_provider` on the next turn, then
   * mirror the admin-configured default into the composer pill. Guarded
   * by `resetInflight` so a double-click cannot fire two PATCH + settings
   * fetch round-trips. Deps are empty because the closure only reads
   * mutable refs (`agentRef`, `chatPanelRef`, `lastPersistedRef`,
   * `resetInflight`) and stable state setters.
   */
  const handleUseDefault = useCallback(async () => {
    if (resetInflight.current) return;
    resetInflight.current = true;
    const agent = agentRef.current;
    const key = agent?.sessionId;
    if (!agent || !key) {
      resetInflight.current = false;
      return;
    }
    setResetError(null);
    // PATCH with explicit nulls to clear the pinned provider/model
    // and let `llm.default_provider` take over on the next turn.
    // The double-option body is what makes the backend distinguish
    // this from a leave-alone call (see #1569).
    //
    // Close the dialog only after the PATCH succeeds — a network
    // failure keeps the dialog open so the error row is visible
    // and the user can retry without chasing a dismissed toast.
    try {
      await api.patch(`/api/v1/chat/sessions/${encodeURIComponent(key)}`, {
        model: null,
        model_provider: null,
        thinking_level: null,
      });
      // Race guard: the user may have switched sessions while
      // the PATCH was in-flight. If so, the composer now
      // reflects a different session and must not be clobbered.
      if (agentRef.current?.sessionId !== key) {
        setModelDialogOpen(false);
        return;
      }
      // Resolve the admin-configured default so the composer pill can
      // read e.g. "minimax: MiniMax-M2.7" instead of pi-web-ui's own
      // hard-coded catalog default (which would otherwise surface a
      // ghost "gemini-2.5-flash-lite" unrelated to rara's config).
      const resolved = await resolveAdminDefaultModel();
      const resolvedModel = resolved
        ? syntheticModel(resolved.provider, resolved.model)
        : syntheticModel(UNKNOWN_MODEL_SENTINEL, UNKNOWN_MODEL_SENTINEL);
      // Re-check the race guard after the settings fetch.
      if (agentRef.current?.sessionId !== key) {
        setModelDialogOpen(false);
        return;
      }
      agent.state.model = resolvedModel;
      lastPersistedRef.current = { model: null, provider: null, thinking: null };
      chatPanelRef.current?.agentInterface?.requestUpdate();
      setModelDialogOpen(false);
    } catch (e: unknown) {
      console.warn('Failed to clear session model override:', e);
      const msg = e instanceof Error ? e.message : String(e);
      setResetError(`Failed to reset model: ${msg}`);
    } finally {
      resetInflight.current = false;
    }
  }, []);

  useEffect(() => {
    if (initRef.current || !containerRef.current) return;
    initRef.current = true;

    const container = containerRef.current;

    void (async () => {
      try {
        // 0. Register rara → pi-mono tool renderer aliases. Must happen before
        //    ChatPanel.setAgent() mounts any messages — the registry is
        //    consulted at render time with no retro-active update.
        registerRaraToolRenderers();
        // Wrap pi-web-ui's built-in `<assistant-message>` so each completed
        // assistant turn gets a "📊 详情" trigger that opens the cascade
        // execution-trace modal. The renderer fires a CustomEvent on the
        // host element (which bubbles up through the light DOM since
        // pi-web-ui's components opt out of shadow DOM) carrying the
        // persisted `seq` (resolved via `assistantSeqByRef`) so the React
        // layer below can call `GET /chat/sessions/{key}/trace` directly.
        registerCascadeAssistantRenderer(() => agentRef.current);

        // 1. Create and initialize the rara storage backend
        const backend = new RaraStorageBackend();
        await backend.init();

        // 2. Create store instances and wire up the backend
        const settings = new SettingsStore();
        settings.setBackend(backend);

        const providerKeys = new ProviderKeysStore();
        providerKeys.setBackend(backend);

        const sessions = new SessionsStore();
        sessions.setBackend(backend);

        const customProviders = new CustomProvidersStore();
        customProviders.setBackend(backend);

        // 3. Create AppStorage and set it as the global instance
        const storage = new AppStorage(settings, providerKeys, sessions, customProviders, backend);
        setAppStorage(storage);

        // 4a. Pull the routable provider catalog in parallel with the
        //     session fetch. Used to reject stale `model_provider` values
        //     persisted by older builds before we touch `agent.state.model`.
        //     Non-fatal if it fails: downstream guards fail-open.
        const providersPromise = api
          .get<ProviderInfo[]>('/api/v1/chat/providers')
          .then((list) => {
            validProvidersRef.current = new Set(list.map((p) => p.id));
          })
          .catch((e: unknown) => {
            console.warn('Failed to load provider catalog for restore guard:', e);
          });

        // 4b. Resolve the active session key before creating the agent.
        //     Prefer the last-active session from localStorage so a
        //     reload lands the user back on whatever they were reading,
        //     falling back to the most recent session, finally creating
        //     a fresh one when nothing exists.
        const storedKey = readStoredSessionKey();
        let initialSession: ChatSession | null = null;
        if (storedKey) {
          try {
            initialSession = await api.get<ChatSession>(
              `/api/v1/chat/sessions/${encodeURIComponent(storedKey)}`,
            );
          } catch {
            // Session was deleted or the key is stale — fall through.
            writeStoredSessionKey(null);
          }
        }
        if (!initialSession) {
          const existingSessions = await api.get<ChatSession[]>(
            '/api/v1/chat/sessions?limit=1&offset=0',
          );
          initialSession = existingSessions[0] ?? null;
        }
        // Block on provider catalog here so the pre-mount restore step has
        // an authoritative allowlist. It's one cheap request; running it
        // serially after the sessions fetch keeps the code simple.
        await providersPromise;
        if (!initialSession) {
          initialSession = await api.post<ChatSession>('/api/v1/chat/sessions', {});
        }
        setActiveSession(initialSession);
        writeStoredSessionKey(initialSession.key);
        setShowWelcome((initialSession.message_count ?? 0) === 0);
        // 5. Create the Agent with rara's WebSocket-backed stream function.
        //    The streamFn reads agent.sessionId at call time to get the active session key.
        const agent: Agent = new Agent({
          streamFn: createRaraStreamFn(
            () => agent.sessionId,
            () => {
              // Surface raw attachments from the latest user turn so the
              // rara-stream adapter can forward document bytes as
              // `file_base64` blocks in addition to pi-mono's client-side
              // extracted text.
              for (let i = agent.state.messages.length - 1; i >= 0; i--) {
                const m = agent.state.messages[i];
                if (!m) continue;
                if (m.role === 'user-with-attachments') {
                  return m.attachments ?? [];
                }
                if (m.role === 'user') return [];
              }
              return [];
            },
            // Feed the agent-live store with every WS frame so the card
            // can render in parallel to pi-chat-panel without opening a
            // second WebSocket (see #1615).
            (sessionKey, event) => liveRunStore.publish(sessionKey, event),
          ),
          convertToLlm: defaultConvertToLlm,
          sessionId: initialSession.key,
        });
        agentRef.current = agent;

        // Restore the initial session's persisted model + thinking-level
        // BEFORE mounting the chat panel, so the composer pill reflects
        // the real selection and `onBeforeSend` does not see pi-agent-core's
        // "unknown" default as the first thing to persist.
        if (
          initialSession.model &&
          initialSession.model_provider &&
          isRoutableProvider(validProvidersRef.current, initialSession.model_provider)
        ) {
          agent.state.model = syntheticModel(initialSession.model_provider, initialSession.model);
          lastPersistedRef.current = {
            model: initialSession.model,
            provider: initialSession.model_provider,
            thinking: initialSession.thinking_level ?? null,
          };
        } else {
          // Unpinned session — seed the composer pill with rara's admin
          // default so it reads "minimax: MiniMax-M2.7" rather than
          // pi-web-ui's hard-coded catalog fallback (`gemini-2.5-*`).
          const resolved = await resolveAdminDefaultModel();
          if (resolved) {
            agent.state.model = syntheticModel(resolved.provider, resolved.model);
          }
        }
        if (initialSession.thinking_level) {
          agent.state.thinkingLevel = initialSession.thinking_level;
        }

        // 6. Mount the ChatPanel custom element
        const chatPanel = document.createElement(
          'pi-chat-panel',
        ) as import('@mariozechner/pi-web-ui').ChatPanel;
        chatPanelRef.current = chatPanel;
        container.appendChild(chatPanel);

        // 7. Wire agent into the panel — skip API key prompt since rara manages
        //    keys server-side, and sync the current model/thinking override to
        //    the backend before every send so the kernel sees the user's
        //    selection for this turn. Overriding `onModelSelect` replaces
        //    pi-mono's `ModelSelector` (which only knows its own hard-coded
        //    `MODELS` catalog) with rara's native dialog sourced from
        //    `/api/v1/settings` — the only place provider ids (`openrouter`,
        //    `kimi`, `minimax`, `glm`, `scnet`, ...) align with rara's kernel
        //    `DriverRegistry`.
        await chatPanel.setAgent(agent, {
          onApiKeyRequired: async () => true,
          onModelSelect: () => setModelDialogOpen(true),
          onBeforeSend: async () => {
            const key = agent.sessionId;
            if (!key) return;
            // The user just committed their first message — no more welcome.
            setShowWelcome(false);
            // Nudge the sidebar to refetch so the fresh session's new
            // title and preview surface in the history list.
            setSidebarRefreshKey((k) => k + 1);
            // Refetch the active session so the backend-assigned title
            // lands in the header above the messages. Fire-and-forget;
            // a retry happens on the next send if the backend hadn't
            // finished assigning a title yet.
            api
              .get<ChatSession>(`/api/v1/chat/sessions/${encodeURIComponent(key)}`)
              .then((fresh) => {
                if (agentRef.current?.sessionId === key) setActiveSession(fresh);
              })
              .catch(() => {
                /* non-fatal */
              });

            // Skip the PATCH when `agent.state.model` is pi-agent-core's
            // placeholder default (id/provider = "unknown"). Persisting it
            // would overwrite any previously saved rara provider with a
            // sentinel the kernel's DriverRegistry cannot route to, which
            // caused the original "LLM provider not configured" failure
            // (see #1554). `isUnknownModel` returns true for null/undefined
            // too, so the subsequent reads are safe without the `?.` guard.
            const picked = !isUnknownModel(agent.state.model);
            const model = picked ? agent.state.model.id : null;
            const provider = picked ? agent.state.model.provider : null;
            const thinking = asThinkingLevel(agent.state.thinkingLevel);

            // Nothing worth persisting.
            if (!model && !thinking) return;

            // Dedup consecutive identical writes — the chat UI round-trips
            // every send through this hook even when the selection hasn't
            // changed, and the PATCH wakes up the session index for nothing.
            const last = lastPersistedRef.current;
            if (
              last &&
              last.model === model &&
              last.provider === provider &&
              last.thinking === thinking
            ) {
              return;
            }

            try {
              await api.patch(`/api/v1/chat/sessions/${encodeURIComponent(key)}`, {
                model,
                model_provider: provider,
                thinking_level: thinking,
              });
              lastPersistedRef.current = { model, provider, thinking };
            } catch (e) {
              console.warn('Failed to persist session LLM override:', e);
            }
          },
        });

        // Model and thinking selectors are enabled by default in ChatPanel.setAgent().
        // Rara delegates model/thinking selection to the user via pi-chat-panel's
        // built-in UI — the chosen model is passed to the backend at stream time.
        //
        // Surface pi-mono's built-in theme toggle in the chat header. Rara's
        // own <ThemeToggle /> is scoped to DashboardLayout (admin pages), so
        // there's no duplicate on the chat page.
        if (chatPanel.agentInterface) {
          chatPanel.agentInterface.showThemeToggle = true;
        }
      } finally {
        // Clear the loading overlay even if init fails (network/CORS/etc.)
        // so the user sees the empty chat panel rather than a spinner forever.
        setIsInitializing(false);
        // Focus the composer on first mount so the caret is live
        // immediately; subsequent session switches do the same via
        // the `switchSession` callback.
        requestAnimationFrame(() => {
          document.querySelector<HTMLTextAreaElement>('textarea')?.focus();
        });
      }
    })();

    return () => {
      // Cleanup: remove the Web Component on unmount
      container.innerHTML = '';
    };
  }, []);

  // Reserve scroll padding inside pi-web-ui's message list equal to the
  // live card's height while a run is active — see hook docstring and
  // `.rara-chat agent-interface .max-w-3xl` rule in `index.css`.
  useLiveCardHeight(liveCardEl, mainEl);

  return (
    <div
      className="rara-chat flex h-screen w-screen"
      data-welcome={showWelcome && !isInitializing ? 'true' : undefined}
    >
      <ChatSidebar
        activeSessionKey={activeSession?.key}
        onSelect={switchSession}
        onNewSession={newSession}
        onOpenSearch={() => setSearchOpen(true)}
        onOpenSettings={() => openSettings()}
        onDeleteSession={handleSessionDeleted}
        refreshKey={sidebarRefreshKey}
      />
      <main ref={setMainEl} className="relative flex min-h-0 min-w-0 flex-1 flex-col">
        {/* Session title header — shows the current conversation's
            title above its messages (kimi-style). Hidden during the
            welcome state since the RARA wordmark already serves as
            the brand marker there. */}
        {activeSession && !showWelcome && !isInitializing && (
          <div className="flex h-11 shrink-0 items-center border-b border-border/30 bg-background/30 px-5 backdrop-blur-sm">
            <span className="truncate text-sm font-medium text-foreground/85">
              {activeSession.title || activeSession.preview || '新对话'}
            </span>
          </div>
        )}
        {/* Chat panel container — takes remaining vertical space. */}
        <div ref={containerRef} className="min-h-0 flex-1 w-full" />
        {/* Live agent card — positioned inline with the pi-web-ui message
            column (same 48rem cap) and anchored just above the composer,
            so it reads as "the next assistant message" while a turn is
            streaming. See `.rara-live-slot` in index.css for placement. */}
        {!isInitializing && (
          <div className="rara-live-slot pointer-events-none absolute z-10 px-2">
            <div ref={setLiveCardEl} className="pointer-events-auto">
              <AgentLiveCard sessionKey={activeSession?.key} />
            </div>
          </div>
        )}
        {/*
          Welcome overlay — rendered above pi-web-ui's empty message list
          when the active session has no messages. Pointer-events-none so
          clicks pass through to the composer below; flipped off the
          moment the user commits their first message (onBeforeSend).
        */}
        {showWelcome && !isInitializing && (
          <div className="pointer-events-none absolute inset-x-0 bottom-[calc(40vh+9rem)] z-10 flex justify-center px-6">
            <h1 className="bg-gradient-to-br from-foreground via-foreground/80 to-foreground/40 bg-clip-text text-6xl font-semibold tracking-[0.2em] text-transparent sm:text-7xl">
              RARA
            </h1>
          </div>
        )}
        {/*
          Voice button floats over pi-web-ui's composer, aligned with
          the paperclip via a calc that tracks the centred composer.
          Lives inside `<main>` so the calc uses main's width (not the
          viewport) — correct when the sidebar takes leftmost space.
        */}
        <VoiceRecorder
          className="voice-float absolute bottom-[29px] z-20 !h-8 !w-8 !rounded-md !bg-transparent !shadow-none hover:!bg-accent"
          getSessionKey={() => agentRef.current?.sessionId}
          onComplete={reloadMessages}
        />
        {/*
          Custom textarea caret (Alma-style): smooth moves + a cool-blue
          comet tail. Mounts after pi-web-ui's composer is in the DOM via
          an internal DOM query since the textarea is owned by a Lit
          custom element we don't ref directly.
        */}
        {!isInitializing && <AlmaCaret measureKey={showWelcome ? 'welcome' : 'chat'} />}
        {/* Initial load overlay — covers the empty container while sessions + agent initialize */}
        {isInitializing && (
          <div className="pointer-events-none absolute inset-0 z-40 flex flex-col items-center justify-center gap-3 bg-background">
            <div className="h-8 w-8 animate-spin rounded-full border-2 border-muted-foreground/30 border-t-muted-foreground" />
            <div className="text-sm text-muted-foreground">Loading sessions…</div>
          </div>
        )}
      </main>
      {/* Rara-native model picker — replaces pi-mono's ModelSelector. */}
      <RaraModelDialog
        open={modelDialogOpen}
        onClose={() => setModelDialogOpen(false)}
        currentProvider={agentRef.current?.state.model?.provider ?? null}
        onSelect={(entry: ProviderInfo) => {
          const agent = agentRef.current;
          if (agent) {
            agent.state.model = syntheticModel(entry.id, entry.default_model, {
              baseUrl: entry.base_url ?? undefined,
            });
            // Force the next onBeforeSend to PATCH even if the new value
            // coincidentally matches the last persisted snapshot (e.g.
            // the user reselects the same provider after a page reload
            // where the snapshot could have drifted from the server).
            lastPersistedRef.current = null;
            chatPanelRef.current?.agentInterface?.requestUpdate();
          }
          setModelDialogOpen(false);
        }}
        resetError={resetError}
        onUseDefault={handleUseDefault}
      />
      <CascadeModal
        open={cascadeOpen}
        trace={cascadeTrace}
        loading={cascadeLoading}
        error={cascadeError}
        onClose={() => setCascadeOpen(false)}
      />
      <SessionSearchDialog
        open={searchOpen}
        onOpenChange={setSearchOpen}
        recentSessions={recentSessions}
        onSelect={(key) => {
          // Look up the session in the currently-cached recents first
          // (no extra request when the user picks a row they can already
          // see in the dialog). Fall back to a GET when the key comes
          // from a search hit the recents list doesn't cover.
          const cached = recentSessions.find((s) => s.key === key);
          if (cached) {
            void switchSession(cached);
            return;
          }
          api
            .get<ChatSession>(`/api/v1/chat/sessions/${encodeURIComponent(key)}`)
            .then((s) => switchSession(s))
            .catch((e: unknown) => {
              console.warn('Failed to open searched session:', e);
            });
        }}
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
