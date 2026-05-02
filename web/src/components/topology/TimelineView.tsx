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

import * as TooltipPrimitive from '@radix-ui/react-tooltip';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { RaraTurnCard } from './RaraTurnCard';
import {
  type TurnCardData,
  buildTurnsFromEvents,
  buildTurnsFromHistory,
  contentToText,
} from './TurnCard';

import type { ChatMessageData } from '@/api/types';
import { useChatModels } from '@/hooks/use-chat-models';
import { useChatSessionWs } from '@/hooks/use-chat-session-ws';
import { useSessionHistory } from '@/hooks/use-session-history';
import type { TopologyEventEntry } from '@/hooks/use-topology-subscription';
import { UserMessageBubble } from '~vendor/components/chat/UserMessageBubble';
import { InputContainer } from '~vendor/components/input/InputContainer';
import { AppShellProvider, type AppShellContextType } from '~vendor/context/AppShellContext';
import { EscapeInterruptProvider } from '~vendor/context/EscapeInterruptContext';

/** Single synthetic connection slug representing rara's effective backend.
 *  rara only ever resolves one default provider server-side today; the
 *  picker just needs *a* connection to attach the model list to. */
const RARA_CONNECTION_SLUG = 'rara';

export interface TimelineViewProps {
  /** Session key whose events should be rendered. Workers (children) flip
   *  this; the prompt editor still sends to the root via `promptSessionKey`. */
  viewSessionKey: string;
  /** Every observed event from the topology subscription. */
  events: TopologyEventEntry[];
  /** Session key the prompt editor sends into. */
  promptSessionKey?: string | null;
  /** When set, REPLACES the live-history message list with this anchor
   *  segment (issue #2040). Live WS turns continue to render at the tail
   *  so a streaming response is still visible while the user is browsing
   *  a chapter. Pass `null` to fall back to the standard "last N
   *  messages" history fetch. */
  segmentMessages?: ChatMessageData[] | null;
}

/**
 * Main-timeline view of an agent's stream of consciousness. Renders user
 * turns (from optimistic local state) and agent turns (from the topology
 * stream) in arrival order, with a craft-style {@link InputContainer}
 * pinned to the bottom of the column.
 *
 * Optimistic user-message rendering: when the user submits, the message
 * is pushed into `userTurns` immediately so it shows up in the timeline
 * before the WS round-trip finishes. The kernel does not echo user
 * prompts back as topology events today, so without this the user would
 * see their text vanish into the input box and only an assistant
 * response appear later.
 */
export function TimelineView({
  viewSessionKey,
  events,
  promptSessionKey,
  segmentMessages,
}: TimelineViewProps) {
  // Per-session ordered user turns. Cleared when the viewed session
  // changes so a new conversation does not inherit a stale prompt list.
  const [userTurnsBySession, setUserTurnsBySession] = useState<
    Record<string, { id: string; text: string; t: number; createdAt: number | null }[]>
  >({});

  const sessionForPrompt = promptSessionKey ?? viewSessionKey;
  const ws = useChatSessionWs(sessionForPrompt);

  // Pull the real model catalog from rara's backend. The vendor input
  // reads its picker entries from `appShellCtx.llmConnections[*].models`,
  // so we wrap a single synthetic "rara" connection around the fetched
  // list. No backend wiring exists yet to honor the user's selection —
  // see the commit body for the follow-up.
  const { data: chatModels } = useChatModels();
  const appShellValue = useMemo<AppShellContextType>(() => {
    const models = (chatModels ?? []).map((m) => ({
      id: m.id,
      name: m.name,
      shortName: m.name,
      description: '',
      provider: 'pi' as const,
      contextWindow: m.context_length,
      supportsThinking: false,
    }));
    const raraConnection = {
      slug: RARA_CONNECTION_SLUG,
      name: 'rara',
      providerType: 'pi',
      authType: 'none',
      models,
      defaultModel: models[0]?.id,
      isAuthenticated: true,
      isDefault: true,
      createdAt: 0,
    };
    // Vendor context expects ~50 fields (workspaces, sessions, callbacks,
    // etc.); the model picker only consumes llmConnections +
    // workspaceDefaultLlmConnection. Cast through unknown so we don't have
    // to fabricate hollow handlers for unused surfaces.
    return {
      llmConnections: [raraConnection],
      workspaceDefaultLlmConnection: RARA_CONNECTION_SLUG,
      workspaces: [],
      activeWorkspaceId: null,
      activeWorkspaceSlug: null,
      pendingPermissions: new Map(),
      pendingCredentials: new Map(),
      sessionOptions: new Map(),
      refreshLlmConnections: async () => {},
      getDraft: () => '',
      getDraftAttachmentRefs: () => [],
      hydrateDraftAttachments: async () => [],
    } as unknown as AppShellContextType;
  }, [chatModels]);

  const defaultModelId = appShellValue.llmConnections[0]?.models?.[0];
  const defaultModelIdString =
    typeof defaultModelId === 'string' ? defaultModelId : defaultModelId?.id;
  const [pickedModel, setPickedModel] = useState<string | undefined>(undefined);
  const [pickedProvider, setPickedProvider] = useState<string | undefined>(undefined);
  const currentModel = pickedModel ?? defaultModelIdString ?? '';

  const history = useSessionHistory(viewSessionKey);
  // When `segmentMessages` is provided by the parent (chapter-strip
  // navigation), it REPLACES the standard history payload. This is the
  // sentinel the spec calls for — live WS turns still render at the
  // tail; only the historical block flips.
  const historyMessages = segmentMessages ?? history.data;
  const historyIsSuccess = history.isSuccess;
  // `dataUpdatedAt` increments on every successful resolution regardless
  // of structural sharing — `historyMessages` would keep the same object
  // reference when react-query refetches identical data, which would
  // silently skip the barrier-snapshot effect after a WS reconnect.
  const historyDataUpdatedAt = history.dataUpdatedAt;

  // Session-filtered slice of the topology subscription buffer. The
  // arrival barrier indexes into THIS array (not the full cross-session
  // buffer) because each `TimelineView` only ever folds events for one
  // session.
  const sessionEvents = useMemo(
    () => events.filter((e) => e.sessionKey === viewSessionKey),
    [events, viewSessionKey],
  );

  // Arrival-time barrier for live/history dedupe.
  //
  // `ChatMessage.seq` (per-tape counter persisted by
  // `tap_entries_to_chat_messages` in
  // `crates/extensions/backend-admin/src/chat/service.rs`) and
  // `TopologyEventEntry.seq` (per-WS-connection frame counter assigned in
  // `use-topology-subscription`) are NOT comparable axes — the WS counter
  // resets to 0 on every reconnect, so any cross-counter filter
  // (`seq > lastHistorySeq`) silently drops live frames after a
  // reconnect. Instead, snapshot the *length* of the session-filtered
  // live buffer at the moment the history fetch resolves: live entries
  // whose buffer index is `< barrier` arrived before history settled and
  // are treated as already represented in the history payload; entries
  // at index `>= barrier` are strictly post-history and rendered live.
  // See `specs/issue-2013-topology-timeline-history.spec.md` Decisions
  // (Live + history dedupe — arrival-barrier, not seq).
  //
  // Reset triggers (per session):
  //   - `viewSessionKey` change — handled implicitly because the map is
  //     keyed by session.
  //   - WS reconnect — detected by the session-filtered buffer length
  //     going backwards (the `events` buffer is rebuilt from `[]` on
  //     `hello`, see `use-topology-subscription`'s `handleFrame`). On
  //     reset we drop the stale barrier; the next successful history
  //     refetch re-snapshots.
  const [barrierBySession, setBarrierBySession] = useState<Record<string, number>>({});
  const lastSessionLengthRef = useRef<Record<string, number>>({});

  useEffect(() => {
    const prevLen = lastSessionLengthRef.current[viewSessionKey] ?? 0;
    const curLen = sessionEvents.length;
    if (curLen < prevLen) {
      // Buffer shrunk → WS reconnect (or session-buffer truncation).
      // Drop the stale barrier so the next successful history fetch
      // re-snapshots, and invalidate the history query so it actually
      // refetches against the new connection.
      setBarrierBySession((prev) => {
        if (!(viewSessionKey in prev)) return prev;
        const next = { ...prev };
        delete next[viewSessionKey];
        return next;
      });
      void history.refetch();
    }
    lastSessionLengthRef.current[viewSessionKey] = curLen;
  }, [sessionEvents.length, viewSessionKey, history]);

  // Snapshot the barrier the first time the history query resolves for
  // this session. We key off `dataUpdatedAt` (a numeric timestamp react-
  // query bumps on every successful resolution) rather than
  // `historyMessages` because react-query's default `structuralSharing`
  // returns a referentially-identical array when a refetch yields the
  // same payload. After a WS reconnect-then-refetch where the persisted
  // history is unchanged, the data reference would not move, the effect
  // would not re-run, the barrier would stay deleted, and the live path
  // would re-render every pre-reconnect event — duplicating with
  // history.
  useEffect(() => {
    if (!historyIsSuccess) return;
    setBarrierBySession((prev) => {
      if (viewSessionKey in prev) return prev;
      return { ...prev, [viewSessionKey]: sessionEvents.length };
    });
    // `sessionEvents.length` intentionally captured at resolution time
    // only — we don't want subsequent length changes to move the
    // barrier.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [historyIsSuccess, historyDataUpdatedAt, viewSessionKey]);

  const historyTurns = useMemo(
    () => (historyMessages ? buildTurnsFromHistory(historyMessages) : []),
    [historyMessages],
  );

  const agentTurns = useMemo(() => {
    // Before the barrier is set (history still pending or errored), the
    // live path is the only source — render every session event so the
    // user sees streaming output immediately.
    const barrier = barrierBySession[viewSessionKey];
    const sliced = barrier === undefined ? sessionEvents : sessionEvents.slice(barrier);
    return buildTurnsFromEvents(sliced.map((e) => ({ seq: e.seq, event: e.event })));
  }, [sessionEvents, viewSessionKey, barrierBySession]);

  // Historical user prompts merged with the optimistic local prompts the
  // editor pushes on submit. Historical entries come first in seq order;
  // optimistic entries follow because they were typed after the page
  // loaded.
  const historyUserBubbles = useMemo<
    { id: string; text: string; t: number; createdAt: number | null }[]
  >(() => {
    if (!historyMessages) return [];
    return historyMessages
      .filter((m) => m.role === 'user')
      .map((m) => {
        const ts = Date.parse(m.created_at);
        return {
          id: `history-user-${String(m.seq)}`,
          text: contentToText(m.content),
          t: 0,
          createdAt: Number.isNaN(ts) ? null : ts,
        };
      })
      .filter((u) => u.text.length > 0);
  }, [historyMessages]);

  // Build a single ordered list of bubbles + turns. History items sort by
  // `createdAt` ascending; items without a timestamp (live agent turns and
  // optimistic local user prompts) trail the history block in arrival
  // order. Each rendered node carries `data-testid="turn-or-bubble"` so
  // the chronological-ordering scenarios can query in document order.
  type Item =
    | { kind: 'bubble'; key: string; createdAt: number | null; text: string }
    | { kind: 'turn'; key: string; createdAt: number | null; turn: TurnCardData };

  const orderedItems = useMemo<Item[]>(() => {
    const optimistic = userTurnsBySession[viewSessionKey] ?? [];
    const items: Item[] = [
      ...historyUserBubbles.map<Item>((u) => ({
        kind: 'bubble',
        key: u.id,
        createdAt: u.createdAt,
        text: u.text,
      })),
      ...historyTurns.map<Item>((t) => ({
        kind: 'turn',
        key: t.id,
        createdAt: t.createdAt,
        turn: t,
      })),
      ...agentTurns.map<Item>((t) => ({
        kind: 'turn',
        key: t.id,
        createdAt: null,
        turn: t,
      })),
      ...optimistic.map<Item>((u) => ({
        kind: 'bubble',
        key: u.id,
        createdAt: null,
        text: u.text,
      })),
    ];
    // Stable sort: items with a `createdAt` come first in ascending order;
    // items without one preserve their input order at the tail. The input
    // order itself already reflects "history then live", so untimed items
    // stay in the right relative order.
    const indexed = items.map((it, idx) => ({ it, idx }));
    indexed.sort((a, b) => {
      const ax = a.it.createdAt;
      const bx = b.it.createdAt;
      if (ax === null && bx === null) return a.idx - b.idx;
      if (ax === null) return 1;
      if (bx === null) return -1;
      if (ax !== bx) return ax - bx;
      return a.idx - b.idx;
    });
    return indexed.map(({ it }) => it);
  }, [historyUserBubbles, historyTurns, agentTurns, userTurnsBySession, viewSessionKey]);

  const isEmpty = orderedItems.length === 0;

  const scrollRef = useRef<HTMLDivElement | null>(null);
  const itemCount = orderedItems.length;
  const lastItemKey = orderedItems.at(-1)?.key;
  const firstItemKey = orderedItems[0]?.key;
  // Segment-mode scroll target: when `segmentMessages` is set, scroll to
  // the FIRST item (the chapter the user clicked) rather than the tail.
  // Re-keying on `segmentMessages` (reference identity) means a fresh
  // chapter click — even one whose first message has the same DOM key
  // as the previous selection — still re-scrolls.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    if (segmentMessages && firstItemKey) {
      el.scrollTop = 0;
    } else {
      el.scrollTop = el.scrollHeight;
    }
  }, [itemCount, lastItemKey, segmentMessages, firstItemKey]);

  const handleSubmit = useCallback(
    (message: string) => {
      const trimmed = message.trim();
      if (!trimmed) return;
      // Forward the picker's current selection as the per-turn override.
      // Empty string means "use whatever the backend default resolves to",
      // which the WS client drops rather than sending a blank `model` field.
      const promptOptions = currentModel
        ? {
            model: currentModel,
            ...(pickedProvider ? { modelProvider: pickedProvider } : {}),
          }
        : undefined;
      const ok = ws.sendPrompt(trimmed, promptOptions);
      if (!ok) return;
      const id = `u-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`;
      setUserTurnsBySession((prev) => {
        const list = prev[viewSessionKey] ?? [];
        return {
          ...prev,
          [viewSessionKey]: [...list, { id, text: trimmed, t: Date.now(), createdAt: null }],
        };
      });
    },
    [ws, viewSessionKey, currentModel, pickedProvider],
  );

  const handleModelChange = useCallback((model: string, connection?: string) => {
    setPickedModel(model);
    setPickedProvider(connection && connection !== RARA_CONNECTION_SLUG ? connection : undefined);
  }, []);

  const handleStop = useCallback(() => {
    ws.sendAbort();
  }, [ws]);

  const isProcessing = ws.status === 'streaming';
  const inputDisabled = ws.status === 'idle' || ws.status === 'closed';

  return (
    // Vendor `TurnCard` (chat) and vendor `InputContainer` (toolbar) both
    // mount radix Tooltips, so a single TooltipProvider wraps the whole
    // view rather than being scoped to the input. EscapeInterruptProvider
    // and AppShellProvider only feed `InputContainer`, but co-locating
    // them here keeps all vendor-context lifetimes on the same level.
    <AppShellProvider value={appShellValue}>
      <EscapeInterruptProvider>
        <TooltipPrimitive.Provider delayDuration={300}>
          <div className="group flex flex-1 min-h-0 flex-col">
            <div
              ref={scrollRef}
              className="scrollbar-hover flex-1 min-h-0 space-y-3 overflow-y-auto pr-1"
            >
              {isEmpty ? (
                <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
                  Waiting for the next turn on{' '}
                  <span className="ml-1 font-mono tabular-nums">{viewSessionKey}</span>…
                </div>
              ) : (
                orderedItems.map((item) =>
                  item.kind === 'bubble' ? (
                    <div key={item.key} data-testid="turn-or-bubble" className="flex justify-end">
                      <UserMessageBubble content={item.text} />
                    </div>
                  ) : (
                    <div key={item.key} data-testid="turn-or-bubble">
                      <RaraTurnCard turn={item.turn} sessionKey={viewSessionKey} />
                    </div>
                  ),
                )
              )}
            </div>
            {history.isError && (
              <div
                role="alert"
                className="mt-2 rounded border border-red-300 bg-red-50 px-3 py-2 text-xs text-red-700 dark:border-red-700 dark:bg-red-950 dark:text-red-300"
              >
                Failed to load conversation history. Live chat still works — refresh to retry.
              </div>
            )}
            {ws.error && (
              <div
                role="alert"
                className="mt-2 rounded border border-red-300 bg-red-50 px-3 py-2 text-xs text-red-700 dark:border-red-700 dark:bg-red-950 dark:text-red-300"
              >
                {ws.error}
              </div>
            )}
            <div className="pt-2">
              <InputContainer
                onSubmit={handleSubmit}
                onStop={handleStop}
                disabled={inputDisabled}
                isProcessing={isProcessing}
                currentModel={currentModel}
                onModelChange={handleModelChange}
                currentConnection={RARA_CONNECTION_SLUG}
                placeholder="Send a message…"
              />
            </div>
          </div>
        </TooltipPrimitive.Provider>
      </EscapeInterruptProvider>
    </AppShellProvider>
  );
}
