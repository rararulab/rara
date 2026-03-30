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
  defaultConvertToLlm,
} from "@mariozechner/pi-web-ui";
import { Agent } from "@mariozechner/pi-agent-core";
import type { AgentMessage } from "@mariozechner/pi-agent-core";
import type { UserMessage, AssistantMessage, TextContent, ImageContent } from "@mariozechner/pi-ai";
import { RaraStorageBackend } from "@/adapters/rara-storage";
import { createRaraStreamFn } from "@/adapters/rara-stream";
import { api } from "@/api/client";
import type { ChatSession, ChatMessageData } from "@/api/types";

/** Strip `<think>...</think>` blocks from assistant text. */
function stripThinkTags(text: string): string {
  return text.replace(/<think>[\s\S]*?<\/think>\s*/g, "").trim();
}

/** Convert rara ChatMessageData to pi-agent-core AgentMessage for display. */
function toAgentMessages(msgs: ChatMessageData[]): AgentMessage[] {
  const result: AgentMessage[] = [];
  for (const m of msgs) {
    const ts = new Date(m.created_at).getTime();

    if (m.role === "user") {
      if (typeof m.content === "string") {
        result.push({ role: "user", content: m.content, timestamp: ts } as UserMessage);
      } else {
        // Map rara content blocks to pi-ai content types, preserving images.
        const piContent: (TextContent | ImageContent)[] = m.content.flatMap(
          (b): (TextContent | ImageContent)[] => {
            if (b.type === "text") return [{ type: "text", text: b.text }];
            if (b.type === "image_base64") {
              const img = b as { type: "image_base64"; media_type: string; data: string };
              return [{ type: "image", mimeType: img.media_type, data: img.data }];
            }
            return [];
          },
        );
        const hasImages = piContent.some((c) => c.type === "image");
        // pi-ai UserMessage accepts string | (TextContent | ImageContent)[];
        // use array form only when images are present to avoid rendering issues.
        const content: string | (TextContent | ImageContent)[] = hasImages
          ? piContent
          : piContent.filter((c): c is TextContent => c.type === "text").map(c => c.text).join("\n");
        result.push({ role: "user", content, timestamp: ts } as UserMessage);
      }
    } else if (m.role === "assistant") {
      const raw =
        typeof m.content === "string"
          ? m.content
          : m.content
              .filter((b): b is { type: "text"; text: string } => b.type === "text")
              .map((b) => b.text)
              .join("\n");
      const text = stripThinkTags(raw);
      const content: TextContent[] = text ? [{ type: "text", text }] : [];
      result.push({
        role: "assistant",
        content,
        api: "messages",
        provider: "anthropic",
        model: "unknown",
        usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: 0, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 } },
        stopReason: "stop",
        timestamp: ts,
      } as AssistantMessage);
    }
  }
  return result;
}

function formatRelativeDate(iso: string): string {
  const diff = Date.now() - new Date(iso).getTime();
  const days = Math.floor(diff / 86_400_000);
  if (days === 0) return "Today";
  if (days === 1) return "Yesterday";
  if (days < 7) return `${days}d ago`;
  return new Date(iso).toLocaleDateString();
}

function SessionListPanel({
  activeKey,
  onSelect,
  onClose,
  onDelete,
  onNew,
}: {
  activeKey: string | undefined;
  onSelect: (s: ChatSession) => void;
  onClose: () => void;
  onDelete: (key: string) => void;
  onNew: () => void;
}) {
  const [sessions, setSessions] = useState<ChatSession[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    api
      .get<ChatSession[]>("/api/v1/chat/sessions?limit=100&offset=0")
      .then(setSessions)
      .catch(() => setSessions([]))
      .finally(() => setLoading(false));

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [onClose]);

  const handleDelete = async (key: string, e: React.MouseEvent) => {
    e.stopPropagation();
    if (!confirm("Delete this session?")) return;
    try {
      await api.del(`/api/v1/chat/sessions/${encodeURIComponent(key)}`);
      setSessions((prev) => prev.filter((s) => s.key !== key));
      onDelete(key);
    } catch {
      /* ignore */
    }
  };

  return (
    <>
      {/* Backdrop */}
      <div
        className="fixed inset-0 z-[60] bg-black/40 backdrop-blur-sm"
        onClick={onClose}
      />
      {/* Panel */}
      <div className="fixed inset-0 right-auto z-[61] flex w-full max-w-80 flex-col border-r border-border bg-background shadow-xl">
        <div className="flex items-center justify-between border-b border-border px-4 py-3">
          <span className="text-sm font-semibold text-foreground">Sessions</span>
          <div className="flex items-center gap-1">
            <button
              onClick={() => { onNew(); onClose(); }}
              className="rounded p-1 text-muted-foreground hover:bg-secondary hover:text-foreground transition-colors cursor-pointer"
              title="New session"
            >
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <path d="M12 5v14M5 12h14" />
              </svg>
            </button>
            <button
              onClick={onClose}
              className="rounded p-1 text-muted-foreground hover:bg-secondary hover:text-foreground transition-colors cursor-pointer"
            >
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <path d="M18 6 6 18M6 6l12 12" />
              </svg>
            </button>
          </div>
        </div>
        <div className="flex-1 overflow-y-auto">
          {loading ? (
            <div className="py-8 text-center text-sm text-muted-foreground">Loading...</div>
          ) : sessions.length === 0 ? (
            <div className="py-8 text-center text-sm text-muted-foreground">No sessions yet</div>
          ) : (
            sessions.map((s) => (
              <div
                key={s.key}
                className={`group flex cursor-pointer items-start gap-3 border-b border-border/50 px-4 py-3 transition-colors hover:bg-secondary/50 ${s.key === activeKey ? "bg-secondary/70" : ""}`}
                onClick={() => { onSelect(s); onClose(); }}
              >
                <div className="min-w-0 flex-1">
                  <div className="truncate text-sm font-medium text-foreground">
                    {stripThinkTags(s.title || s.preview || "New conversation")}
                  </div>
                  {s.title && s.preview && (
                    <div className="mt-0.5 truncate text-xs text-muted-foreground">
                      {stripThinkTags(s.preview)}
                    </div>
                  )}
                  <div className="mt-1 text-[11px] text-muted-foreground/70">
                    {formatRelativeDate(s.updated_at)}
                  </div>
                </div>
                <button
                  className="mt-0.5 shrink-0 rounded p-1 text-destructive opacity-0 transition-opacity hover:bg-destructive/10 group-hover:opacity-100 cursor-pointer"
                  onClick={(e) => handleDelete(s.key, e)}
                  title="Delete"
                >
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                    <path d="M3 6h18M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2" />
                  </svg>
                </button>
              </div>
            ))
          )}
        </div>
      </div>
    </>
  );
}

/**
 * Fullscreen wrapper that mounts pi-web-ui's <pi-chat-panel> Web Component,
 * wiring it up to rara's storage backend and WebSocket stream function.
 */
export default function PiChat() {
  const containerRef = useRef<HTMLDivElement>(null);
  const initRef = useRef(false);
  const agentRef = useRef<Agent | null>(null);
  const chatPanelRef = useRef<import("@mariozechner/pi-web-ui").ChatPanel | null>(null);
  const [showSessionList, setShowSessionList] = useState(false);

  /** Switch the agent to a different session, loading its history. */
  const switchSession = useCallback(async (session: ChatSession) => {
    const agent = agentRef.current;
    if (!agent) return;
    agent.clearMessages();
    agent.sessionId = session.key;
    try {
      const msgs = await api.get<ChatMessageData[]>(
        `/api/v1/chat/sessions/${encodeURIComponent(session.key)}/messages?limit=200`,
      );
      const agentMsgs = toAgentMessages(msgs);
      if (agentMsgs.length > 0) {
        agent.replaceMessages(agentMsgs);
      }
    } catch {
      /* session may have no messages yet */
    }
    // Always trigger re-render after switching — even for empty sessions
    // so cleared messages are reflected in the UI.
    chatPanelRef.current?.agentInterface?.requestUpdate();
  }, []);

  /** Create a new empty session and switch to it. */
  const newSession = useCallback(async () => {
    const created = await api.post<ChatSession>("/api/v1/chat/sessions", {});
    switchSession(created);
  }, [switchSession]);

  /** Handle session deletion from the panel. */
  const handleSessionDeleted = useCallback(
    (deletedKey: string) => {
      const agent = agentRef.current;
      if (agent && agent.sessionId === deletedKey) {
        newSession();
      }
    },
    [newSession],
  );

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
      // 5. Create the Agent with rara's WebSocket-backed stream function.
      //    The streamFn reads agent.sessionId at call time to get the active session key.
      const agent: Agent = new Agent({
        streamFn: createRaraStreamFn(() => agent.sessionId),
        convertToLlm: defaultConvertToLlm,
        sessionId: initialSession.key,
      });
      agentRef.current = agent;

      // 6. Mount the ChatPanel custom element
      const chatPanel = document.createElement("pi-chat-panel") as import("@mariozechner/pi-web-ui").ChatPanel;
      chatPanelRef.current = chatPanel;
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
    <div className="relative h-screen w-screen">
      {/* Sessions button — fixed top-left */}
      <button
        onClick={() => setShowSessionList(true)}
        className="absolute left-2 top-2 z-50 flex h-11 w-11 items-center justify-center rounded-md bg-background/80 text-muted-foreground shadow-sm backdrop-blur hover:bg-secondary hover:text-foreground transition-colors cursor-pointer"
        title="Sessions"
      >
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
          <path d="M3 12h18M3 6h18M3 18h18" />
        </svg>
      </button>
      {/* Chat panel container */}
      <div ref={containerRef} className="h-full w-full" />
      {/* Session list slide-over */}
      {showSessionList && (
        <SessionListPanel
          activeKey={agentRef.current?.sessionId}
          onSelect={switchSession}
          onClose={() => setShowSessionList(false)}
          onDelete={handleSessionDeleted}
          onNew={newSession}
        />
      )}
    </div>
  );
}
