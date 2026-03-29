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

import { useEffect, useRef, useCallback, useState } from "react";
import {
  AppStorage,
  setAppStorage,
  SessionsStore,
  SettingsStore,
  ProviderKeysStore,
  CustomProvidersStore,
  SessionListDialog,
} from "@mariozechner/pi-web-ui";
import { Agent } from "@mariozechner/pi-agent-core";
import { RaraStorageBackend } from "@/adapters/rara-storage";
import { createRaraStreamFn } from "@/adapters/rara-stream";
import { api } from "@/api/client";
import type { ChatSession } from "@/api/types";

/**
 * Fullscreen wrapper that mounts pi-web-ui's <pi-chat-panel> Web Component,
 * wiring it up to rara's storage backend and WebSocket stream function.
 */
export default function PiChat() {
  const containerRef = useRef<HTMLDivElement>(null);
  const initRef = useRef(false);
  const agentRef = useRef<Agent | null>(null);
  const [sessionTitle, setSessionTitle] = useState<string | null>(null);

  /** Switch the agent to a different session. */
  const switchSession = useCallback((sessionKey: string, title?: string | null) => {
    const agent = agentRef.current;
    if (!agent) return;
    agent.clearMessages();
    agent.sessionId = sessionKey;
    setSessionTitle(title ?? null);
  }, []);

  /** Create a new empty session and switch to it. */
  const newSession = useCallback(async () => {
    const created = await api.post<ChatSession>("/api/v1/chat/sessions", {});
    switchSession(created.key, null);
  }, [switchSession]);

  /** Open the session list dialog. */
  const openSessionList = useCallback(() => {
    SessionListDialog.open(
      (sessionId: string) => {
        // Fetch session metadata to get the title
        api.get<ChatSession[]>("/api/v1/chat/sessions?limit=100&offset=0").then(
          (sessions) => {
            const found = sessions.find((s) => s.key === sessionId);
            switchSession(sessionId, found?.title);
          },
          () => switchSession(sessionId),
        );
      },
      (_deletedId: string) => {
        // Session deleted — if it's the active one, create a new session
        const agent = agentRef.current;
        if (agent && agent.sessionId === _deletedId) {
          newSession();
        }
      },
    );
  }, [switchSession, newSession]);

  useEffect(() => {
    if (initRef.current || !containerRef.current) return;
    initRef.current = true;

    const container = containerRef.current;

    (async () => {
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
      const storage = new AppStorage(
        settings,
        providerKeys,
        sessions,
        customProviders,
        backend,
      );
      setAppStorage(storage);

      // 4. Resolve the active session key before creating the agent.
      //    Use the most recent existing session or create a new one.
      const existingSessions = await api.get<ChatSession[]>(
        "/api/v1/chat/sessions?limit=1&offset=0",
      );
      let initialSession: ChatSession;
      if (existingSessions.length > 0) {
        initialSession = existingSessions[0];
      } else {
        initialSession = await api.post<ChatSession>("/api/v1/chat/sessions", {});
      }
      setSessionTitle(initialSession.title);

      // 5. Create the Agent with rara's WebSocket-backed stream function.
      //    The streamFn reads agent.sessionId at call time to get the active session key.
      const agent: Agent = new Agent({
        streamFn: createRaraStreamFn(() => agent.sessionId),
        sessionId: initialSession.key,
      });
      agentRef.current = agent;

      // 6. Mount the ChatPanel custom element
      const chatPanel = document.createElement("pi-chat-panel") as import("@mariozechner/pi-web-ui").ChatPanel;
      container.appendChild(chatPanel);

      // 7. Wire agent into the panel — skip API key prompt since rara manages keys server-side
      await chatPanel.setAgent(agent, {
        onApiKeyRequired: async () => true,
      });

      // 8. Hide model/thinking selectors — rara manages these server-side
      if (chatPanel.agentInterface) {
        chatPanel.agentInterface.enableModelSelector = false;
        chatPanel.agentInterface.enableThinkingSelector = false;
      }
    })();

    return () => {
      // Cleanup: remove the Web Component on unmount
      container.innerHTML = "";
    };
  }, []);

  return (
    <div className="flex h-screen w-screen flex-col">
      {/* Session toolbar */}
      <div className="flex h-11 shrink-0 items-center justify-between border-b border-border px-4">
        <button
          onClick={openSessionList}
          className="truncate text-sm font-medium text-foreground hover:text-foreground/80 transition-colors cursor-pointer"
          title="Switch session"
        >
          {sessionTitle || "New conversation"}
        </button>
        <button
          onClick={newSession}
          className="flex items-center gap-1 rounded-md px-2 py-1 text-xs text-muted-foreground hover:bg-secondary hover:text-foreground transition-colors cursor-pointer"
          title="New session"
        >
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <path d="M12 5v14M5 12h14" />
          </svg>
          New
        </button>
      </div>
      {/* Chat panel container */}
      <div ref={containerRef} className="min-h-0 flex-1" />
    </div>
  );
}
