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

import { TurnCard, buildTurnsFromEvents } from './TurnCard';

import { useChatModels } from '@/hooks/use-chat-models';
import { useChatSessionWs } from '@/hooks/use-chat-session-ws';
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
export function TimelineView({ viewSessionKey, events, promptSessionKey }: TimelineViewProps) {
  // Per-session ordered user turns. Cleared when the viewed session
  // changes so a new conversation does not inherit a stale prompt list.
  const [userTurnsBySession, setUserTurnsBySession] = useState<
    Record<string, { id: string; text: string; t: number }[]>
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
  const currentModel = pickedModel ?? defaultModelIdString ?? '';

  const agentTurns = useMemo(() => {
    const sessionEvents = events
      .filter((e) => e.sessionKey === viewSessionKey)
      .map((e) => ({ seq: e.seq, event: e.event }));
    return buildTurnsFromEvents(sessionEvents);
  }, [events, viewSessionKey]);

  const userTurns = userTurnsBySession[viewSessionKey] ?? [];

  // Interleaving by wall-clock arrival time would require timestamps on
  // agent turns, which the current TurnCard reducer doesn't track. Until
  // that lands, we put all user prompts above the agent turns for the
  // session — which matches the craft layout when you first open a new
  // conversation. Subsequent prompts will appear after the latest agent
  // turn in practice because agent turns auto-scroll on append.
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const lastAgentTurnId = agentTurns.at(-1)?.id;
  const userCount = userTurns.length;
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
  }, [agentTurns.length, lastAgentTurnId, userCount]);

  const handleSubmit = useCallback(
    (message: string) => {
      const trimmed = message.trim();
      if (!trimmed) return;
      // Forward the picker's current selection as the per-turn override.
      // Empty string means "use whatever the backend default resolves to",
      // which the WS client drops rather than sending a blank `model` field.
      const ok = ws.sendPrompt(trimmed, currentModel ? { model: currentModel } : undefined);
      if (!ok) return;
      const id = `u-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`;
      setUserTurnsBySession((prev) => {
        const list = prev[viewSessionKey] ?? [];
        return { ...prev, [viewSessionKey]: [...list, { id, text: trimmed, t: Date.now() }] };
      });
    },
    [ws, viewSessionKey, currentModel],
  );

  const handleStop = useCallback(() => {
    ws.sendAbort();
  }, [ws]);

  const isProcessing = ws.status === 'streaming';
  const inputDisabled = ws.status === 'idle' || ws.status === 'closed';

  return (
    <div className="flex flex-1 min-h-0 flex-col">
      <div ref={scrollRef} className="flex-1 min-h-0 space-y-3 overflow-y-auto pr-1">
        {agentTurns.length === 0 && userTurns.length === 0 ? (
          <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
            Waiting for the next turn on <span className="ml-1 font-mono">{viewSessionKey}</span>…
          </div>
        ) : (
          <>
            {userTurns.map((u) => (
              <div key={u.id} className="flex justify-end">
                <UserMessageBubble content={u.text} />
              </div>
            ))}
            {agentTurns.map((turn) => (
              <TurnCard key={turn.id} turn={turn} />
            ))}
          </>
        )}
      </div>
      <div className="pt-2">
        {/* Vendor InputContainer reaches for an EscapeInterruptProvider
         *  (double-Esc interrupt UX) and a radix TooltipProvider (toolbar
         *  hover hints). Wrap locally rather than at the App root so the
         *  provider lifetime tracks the timeline view, and rara's other
         *  pages stay free of vendor-side context noise. */}
        <AppShellProvider value={appShellValue}>
          <EscapeInterruptProvider>
            <TooltipPrimitive.Provider delayDuration={300}>
              <InputContainer
                onSubmit={handleSubmit}
                onStop={handleStop}
                disabled={inputDisabled}
                isProcessing={isProcessing}
                currentModel={currentModel}
                onModelChange={setPickedModel}
                currentConnection={RARA_CONNECTION_SLUG}
                placeholder="Send a message…"
              />
            </TooltipPrimitive.Provider>
          </EscapeInterruptProvider>
        </AppShellProvider>
      </div>
    </div>
  );
}
