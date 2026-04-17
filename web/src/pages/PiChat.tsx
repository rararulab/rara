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
  ProvidersModelsTab,
  ProxyTab,
  SettingsDialog,
  type Attachment,
  type UserMessageWithAttachments,
} from "@mariozechner/pi-web-ui";
import { Agent } from "@mariozechner/pi-agent-core";
import type { AgentMessage } from "@mariozechner/pi-agent-core";
// Importing the extract-document tool from pi-web-ui triggers the
// module-level `registerToolRenderer("extract_document", ...)` side
// effect so pi-mono can render server-triggered document-extraction tool
// calls in chat.
import { extractDocumentTool } from "@mariozechner/pi-web-ui";

// Reference the tool so Vite's tree-shaker keeps the module (and its
// `registerToolRenderer` side effect) in the bundle. The actual tool
// object is executed server-side; the renderer is what matters here.
void extractDocumentTool;
import type {
  UserMessage,
  AssistantMessage,
  TextContent,
  ThinkingContent,
  ToolCall,
  ToolResultMessage,
} from "@mariozechner/pi-ai";
import { RaraStorageBackend } from "@/adapters/rara-storage";
import { createRaraStreamFn } from "@/adapters/rara-stream";
import { registerRaraToolRenderers } from "@/tools/rara-tool-renderers";
import { api } from "@/api/client";
import type { ChatSession, ChatMessageData, ThinkingLevel } from "@/api/types";
import { useNavigate } from "react-router";
import { VoiceRecorder } from "@/components/VoiceRecorder";
import {
  RaraModelDialog,
  type RaraProviderEntry,
} from "@/components/RaraModelDialog";

/** Strip `<think>...</think>` blocks — used only for UI preview/title text. */
function stripForPreview(text: string): string {
  return text.replace(/<think>[\s\S]*?<\/think>\s*/g, "").trim();
}

/**
 * The rara backend accepts the same six buckets pi-mono exposes
 * (`off | minimal | low | medium | high | xhigh`), so the chat-panel
 * selector round-trips verbatim. This guard just narrows the type.
 */
function asThinkingLevel(level: string | undefined): ThinkingLevel | null {
  switch (level) {
    case "off":
    case "minimal":
    case "low":
    case "medium":
    case "high":
    case "xhigh":
      return level;
    default:
      return null;
  }
}

/**
 * Detect whether a tool-result payload represents a failure. Mirrors the
 * backend's `is_failure_result` in `crates/app/src/tools/artifacts.rs`: a
 * bare string starting with `Error:` (pi-mono convention) or a JSON object
 * with an `error` key (kernel-serialized anyhow error).
 */
function isToolFailure(text: string): boolean {
  const trimmed = text.trimStart();
  if (trimmed.startsWith("Error:")) return true;
  try {
    const parsed = JSON.parse(trimmed);
    return (
      typeof parsed === "object"
      && parsed !== null
      && !Array.isArray(parsed)
      && "error" in parsed
    );
  } catch {
    return false;
  }
}

/**
 * Build a pi-ai `Model<any>`-shaped object from rara's own provider
 * data. pi-chat-panel uses `agent.state.model` only for UI display (the
 * model-name pill above the composer); actual streaming bypasses pi-ai
 * entirely and rides on rara's WebSocket, so the synthesized fields
 * (`api`, `baseUrl`, `cost`, `contextWindow`) need only be structurally
 * valid — their values never hit the wire.
 */
function syntheticModel(
  providerId: string,
  modelId: string,
  options?: { baseUrl?: string; contextWindow?: number; name?: string },
): unknown {
  return {
    id:            modelId,
    name:          options?.name ?? `${providerId} / ${modelId}`,
    api:           "openai-completions",
    provider:      providerId,
    baseUrl:       options?.baseUrl ?? "",
    reasoning:     false,
    input:         ["text", "image"],
    cost:          { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    contextWindow: options?.contextWindow ?? 128_000,
    maxTokens:     4096,
  };
}

function mimeToFilename(mimeType: string, index: number): string {
  const ext = mimeType.split("/")[1] || "bin";
  return `session-image-${index + 1}.${ext}`;
}

/** Zeroed usage — rara tracks usage server-side. */
const EMPTY_USAGE = {
  input: 0, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: 0,
  cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
};

/**
 * Parse assistant text into ThinkingContent + TextContent blocks.
 * `<think>reasoning</think>answer` → [{type:"thinking",...}, {type:"text",...}]
 */
function parseAssistantContent(
  raw: string,
): (TextContent | ThinkingContent)[] {
  const blocks: (TextContent | ThinkingContent)[] = [];
  const re = /<think>([\s\S]*?)<\/think>/g;
  let cursor = 0;
  let match: RegExpExecArray | null;

  while ((match = re.exec(raw)) !== null) {
    // Text before this <think> block
    const before = raw.slice(cursor, match.index).trim();
    if (before) blocks.push({ type: "text", text: before });
    // Thinking content
    const thinking = match[1].trim();
    if (thinking) blocks.push({ type: "thinking", thinking });
    cursor = match.index + match[0].length;
  }

  // Remaining text after the last </think>
  const tail = raw.slice(cursor).trim();
  if (tail) blocks.push({ type: "text", text: tail });

  return blocks;
}

/** Convert rara ChatMessageData to pi-agent-core AgentMessage for display. */
function toAgentMessages(msgs: ChatMessageData[]): AgentMessage[] {
  const result: AgentMessage[] = [];
  // Track the last assistant message so "tool" role messages can attach ToolCall items.
  let lastAssistant: AssistantMessage | null = null;

  for (const m of msgs) {
    const ts = new Date(m.created_at).getTime();

    if (m.role === "user") {
      lastAssistant = null;
      if (typeof m.content === "string") {
        result.push({ role: "user", content: m.content, timestamp: ts } as UserMessage);
      } else {
        const text = m.content
          .filter((b): b is { type: "text"; text: string } => b.type === "text")
          .map((b) => b.text)
          .join("\n");
        const attachments: Attachment[] = m.content.flatMap((b, index): Attachment[] => {
          if (b.type !== "image_base64") return [];
          return [{
            id: `${m.seq}-image-${index}`,
            type: "image",
            fileName: mimeToFilename(b.media_type, index),
            mimeType: b.media_type,
            size: Math.floor((b.data.length * 3) / 4),
            content: b.data,
            preview: b.data,
          }];
        });

        if (attachments.length > 0) {
          result.push({
            role: "user-with-attachments",
            content: text,
            attachments,
            timestamp: ts,
          } as UserMessageWithAttachments as AgentMessage);
        } else {
          result.push({ role: "user", content: text, timestamp: ts } as UserMessage);
        }
      }
    } else if (m.role === "assistant") {
      const raw =
        typeof m.content === "string"
          ? m.content
          : m.content
              .filter((b): b is { type: "text"; text: string } => b.type === "text")
              .map((b) => b.text)
              .join("\n");
      const content: (TextContent | ThinkingContent | ToolCall)[] =
        parseAssistantContent(raw);
      // Surface persisted tool-call requests so pi-web-ui reducers (and the
      // artifacts panel's reconstructFromMessages) can see them.
      if (m.tool_calls && m.tool_calls.length > 0) {
        for (const tc of m.tool_calls) {
          const args =
            tc.arguments && typeof tc.arguments === "object"
              ? (tc.arguments as Record<string, unknown>)
              : {};
          content.push({
            type: "toolCall",
            id: tc.id,
            name: tc.name,
            arguments: args,
          });
        }
      }
      const assistant: AssistantMessage = {
        role: "assistant",
        content,
        api: "messages",
        provider: "anthropic",
        model: "unknown",
        usage: EMPTY_USAGE,
        stopReason: "stop",
        timestamp: ts,
      };
      lastAssistant = assistant;
      result.push(assistant);
    } else if (m.role === "tool") {
      // Tool call from the assistant — attach as ToolCall to the last AssistantMessage.
      if (lastAssistant && m.tool_call_id && m.tool_name) {
        let args: Record<string, unknown> = {};
        try {
          const raw = typeof m.content === "string" ? m.content : JSON.stringify(m.content);
          args = JSON.parse(raw);
        } catch { /* use empty args */ }
        const toolCall: ToolCall = {
          type: "toolCall",
          id: m.tool_call_id,
          name: m.tool_name,
          arguments: args,
        };
        (lastAssistant.content as (TextContent | ThinkingContent | ToolCall)[]).push(toolCall);
      }
    } else if (m.role === "tool_result") {
      // Tool result — emit as a separate ToolResultMessage. Preserve the
      // backend's failure markers so ArtifactsPanel.reconstructFromMessages
      // (which only replays successful ops) skips failed calls on reload.
      // The kernel serializes failures in two shapes: a bare string starting
      // with "Error:" (pi-mono convention) and JSON objects with an `error`
      // key (produced by the anyhow -> ToolOutput path).
      if (m.tool_call_id && m.tool_name) {
        const text = typeof m.content === "string"
          ? m.content
          : m.content
              .filter((b): b is { type: "text"; text: string } => b.type === "text")
              .map((b) => b.text)
              .join("\n");
        const toolResult: ToolResultMessage = {
          role: "toolResult",
          toolCallId: m.tool_call_id,
          toolName: m.tool_name,
          content: text ? [{ type: "text", text }] : [],
          isError: isToolFailure(text),
          timestamp: ts,
        };
        result.push(toolResult as AgentMessage);
      }
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
  onNavigate,
}: {
  activeKey: string | undefined;
  onSelect: (s: ChatSession) => void;
  onClose: () => void;
  onDelete: (key: string) => void;
  onNew: () => void;
  onNavigate: (path: string) => void;
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
                    {stripForPreview(s.title || s.preview || "New conversation")}
                  </div>
                  {s.title && s.preview && (
                    <div className="mt-0.5 truncate text-xs text-muted-foreground">
                      {stripForPreview(s.preview)}
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
        {/*
          Server-wide admin settings (MCP servers, agent manifests, skills,
          kernel config). Chat-user preferences (provider API keys, proxy,
          theme, thinking default) live in pi-mono's SettingsDialog opened
          from the gear icon in the top bar.
        */}
        <div className="border-t border-border px-4 py-3">
          <button
            onClick={() => { onNavigate("/settings"); onClose(); }}
            className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-sm text-muted-foreground hover:bg-secondary hover:text-foreground transition-colors cursor-pointer"
          >
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M3 3h7v7H3zM14 3h7v7h-7zM14 14h7v7h-7zM3 14h7v7H3z" />
            </svg>
            Admin
          </button>
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
  const [isInitializing, setIsInitializing] = useState(true);
  const [modelDialogOpen, setModelDialogOpen] = useState(false);
  const navigate = useNavigate();

  /** Switch the agent to a different session, loading its history. */
  const switchSession = useCallback(async (session: ChatSession) => {
    const agent = agentRef.current;
    if (!agent) return;
    agent.clearMessages();
    agent.sessionId = session.key;

    // Restore the session's persisted model + thinking-level so the
    // model pill in the composer reflects the last settings used for
    // this conversation. We build a synthetic pi-ai Model rather than
    // looking the pair up in pi-ai's catalog — rara's provider ids
    // (`kimi`, `openrouter`, `scnet`, ...) do not exist there.
    if (session.model && session.model_provider) {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      agent.state.model = syntheticModel(session.model_provider, session.model) as any;
    }
    if (session.thinking_level) {
      agent.state.thinkingLevel = session.thinking_level;
    }

    try {
      const msgs = await api.get<ChatMessageData[]>(
        `/api/v1/chat/sessions/${encodeURIComponent(session.key)}/messages?limit=200`,
      );
      const agentMsgs = toAgentMessages(msgs);
      if (agentMsgs.length > 0) {
        agent.replaceMessages(agentMsgs);
      }
      // Rebuild the artifacts panel from the same message list so switching
      // back to a session restores every previously-created artifact.
      await chatPanelRef.current?.artifactsPanel?.reconstructFromMessages(
        agentMsgs,
      );
    } catch {
      /* session may have no messages yet */
    }
    // Always trigger re-render after switching — even for empty sessions
    // so cleared messages are reflected in the UI.
    chatPanelRef.current?.agentInterface?.requestUpdate();
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
        agentMsgs,
      );
      chatPanelRef.current?.agentInterface?.requestUpdate();
    } catch {
      /* ignore */
    }
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
      try {
      // 0. Register rara → pi-mono tool renderer aliases. Must happen before
      //    ChatPanel.setAgent() mounts any messages — the registry is
      //    consulted at render time with no retro-active update.
      registerRaraToolRenderers();

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
        streamFn: createRaraStreamFn(
          () => agent.sessionId,
          () => {
            // Surface raw attachments from the latest user turn so the
            // rara-stream adapter can forward document bytes as
            // `file_base64` blocks in addition to pi-mono's client-side
            // extracted text.
            for (let i = agent.state.messages.length - 1; i >= 0; i--) {
              const m = agent.state.messages[i];
              if (m.role === "user-with-attachments") {
                return (m as UserMessageWithAttachments).attachments;
              }
              if (m.role === "user") return [];
            }
            return [];
          },
        ),
        convertToLlm: defaultConvertToLlm,
        sessionId: initialSession.key,
      });
      agentRef.current = agent;

      // 6. Mount the ChatPanel custom element
      const chatPanel = document.createElement("pi-chat-panel") as import("@mariozechner/pi-web-ui").ChatPanel;
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
          const model = agent.state.model?.id ?? null;
          const model_provider = agent.state.model?.provider ?? null;
          const thinking_level = asThinkingLevel(agent.state.thinkingLevel);
          // Skip the PATCH when nothing would change.
          if (!model && !thinking_level) return;
          try {
            await api.patch(`/api/v1/chat/sessions/${encodeURIComponent(key)}`, {
              model,
              model_provider,
              thinking_level,
            });
          } catch (e) {
            console.warn("Failed to persist session LLM override:", e);
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
      {/*
        Settings button — opens pi-mono's SettingsDialog (provider API keys,
        custom providers, proxy). Rara's server-wide admin config (MCP servers,
        agent manifests, kernel config) still lives at `/settings` — reachable
        from the session-list footer.
      */}
      <button
        onClick={() =>
          SettingsDialog.open([new ProvidersModelsTab(), new ProxyTab()])
        }
        className="absolute left-14 top-2 z-50 flex h-11 w-11 items-center justify-center rounded-md bg-background/80 text-muted-foreground shadow-sm backdrop-blur hover:bg-secondary hover:text-foreground transition-colors cursor-pointer"
        title="Settings"
      >
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
          <circle cx="12" cy="12" r="3" />
          <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
        </svg>
      </button>
      {/* Voice recorder button — fixed top-right */}
      <div className="absolute right-2 top-2 z-50">
        <VoiceRecorder
          getSessionKey={() => agentRef.current?.sessionId}
          onComplete={reloadMessages}
        />
      </div>
      {/* Chat panel container */}
      <div ref={containerRef} className="h-full w-full" />
      {/* Initial load overlay — covers the empty container while sessions + agent initialize */}
      {isInitializing && (
        <div className="pointer-events-none absolute inset-0 z-40 flex flex-col items-center justify-center gap-3 bg-background">
          <div className="h-8 w-8 animate-spin rounded-full border-2 border-muted-foreground/30 border-t-muted-foreground" />
          <div className="text-sm text-muted-foreground">Loading sessions…</div>
        </div>
      )}
      {/* Session list slide-over */}
      {showSessionList && (
        <SessionListPanel
          activeKey={agentRef.current?.sessionId}
          onSelect={switchSession}
          onClose={() => setShowSessionList(false)}
          onDelete={handleSessionDeleted}
          onNew={newSession}
          onNavigate={navigate}
        />
      )}
      {/* Rara-native model picker — replaces pi-mono's ModelSelector. */}
      <RaraModelDialog
        open={modelDialogOpen}
        onClose={() => setModelDialogOpen(false)}
        currentProvider={agentRef.current?.state.model?.provider ?? null}
        onSelect={(entry: RaraProviderEntry) => {
          const agent = agentRef.current;
          if (agent) {
            // eslint-disable-next-line @typescript-eslint/no-explicit-any
            agent.state.model = syntheticModel(entry.id, entry.default_model, {
              baseUrl: entry.base_url,
            }) as any;
            chatPanelRef.current?.agentInterface?.requestUpdate();
          }
          setModelDialogOpen(false);
        }}
      />
    </div>
  );
}
