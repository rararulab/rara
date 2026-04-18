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
  // Stub stores only — rara's admin settings modal is the real source of
  // truth for provider keys and custom providers (see #1581).
  ProviderKeysStore,
  CustomProvidersStore,
  defaultConvertToLlm,
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
import { api, settingsApi } from "@/api/client";
import type { ChatSession, ChatMessageData, ThinkingLevel } from "@/api/types";
import { VoiceRecorder } from "@/components/VoiceRecorder";
import { RaraModelDialog } from "@/components/RaraModelDialog";
import { useSettingsModal } from "@/components/settings/SettingsModalProvider";
import type { ProviderInfo } from "@/api/types";
import { UNKNOWN_MODEL_SENTINEL, isUnknownModel, syntheticModel } from "@/lib/synthetic-model";

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
  onOpenAdmin,
}: {
  activeKey: string | undefined;
  onSelect: (s: ChatSession) => void;
  onClose: () => void;
  onDelete: (key: string) => void;
  onNew: () => void;
  onOpenAdmin: () => void;
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
          kernel config, provider keys). Opens the floating settings modal
          in place — no navigation away from the chat.
        */}
        <div className="border-t border-border px-4 py-3">
          <button
            onClick={() => { onOpenAdmin(); onClose(); }}
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
  // Tracks the last successfully-persisted (model, provider, thinking)
  // triple so onBeforeSend can skip no-op PATCHes on every send.
  const lastPersistedRef = useRef<{ model: string | null; provider: string | null; thinking: string | null } | null>(null);
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
  const [showSessionList, setShowSessionList] = useState(false);
  const [isInitializing, setIsInitializing] = useState(true);
  const [modelDialogOpen, setModelDialogOpen] = useState(false);
  const [resetError, setResetError] = useState<string | null>(null);
  // `true` when the active session has no messages — we render a welcome
  // overlay in that window so the chat page isn't just an input box on
  // empty canvas. Flipped off on the first send and on session switches
  // that land on a populated session.
  const [showWelcome, setShowWelcome] = useState(true);
  const { openSettings } = useSettingsModal();

  // Clear any stale reset-error banner whenever the model dialog is
  // closed — regardless of close path (backdrop click, successful
  // select, successful reset). Co-locating the clear here prevents the
  // banner from leaking into the next dialog opening.
  useEffect(() => {
    if (!modelDialogOpen) setResetError(null);
  }, [modelDialogOpen]);

  /** Switch the agent to a different session, loading its history. */
  const switchSession = useCallback(async (session: ChatSession) => {
    const agent = agentRef.current;
    if (!agent) return;
    agent.clearMessages();
    agent.sessionId = session.key;
    setShowWelcome((session.message_count ?? 0) === 0);

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
    }
    if (session.thinking_level) {
      agent.state.thinkingLevel = session.thinking_level;
    }
    // Reset the dedup ref to match the session that was just loaded so
    // onBeforeSend correctly re-PATCHes if the user changes selection
    // away from the restored values, and skips the identity write
    // otherwise.
    lastPersistedRef.current = {
      model:    session.model ?? null,
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
        model:          null,
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
      // Resolve the admin-configured default so the composer
      // pill can read e.g. "codex: codex-mini" instead of the
      // "unknown" sentinel. PiChat does not already consume
      // react-query so we fire a one-shot request — failures
      // here are non-fatal and fall back to the sentinel.
      let resolvedModel = syntheticModel(
        UNKNOWN_MODEL_SENTINEL,
        UNKNOWN_MODEL_SENTINEL,
      );
      try {
        const settings = await settingsApi.list();
        const provider = settings["llm.default_provider"]?.trim();
        const model = provider
          ? settings[`llm.providers.${provider}.default_model`]?.trim()
          : undefined;
        if (provider && model) {
          resolvedModel = syntheticModel(provider, model);
        } else if (provider) {
          // Admin set a default provider but forgot the paired
          // `default_model` key — surface during development so the
          // misconfig is caught before users see the unknown sentinel.
          console.warn(
            `Admin default provider \`${provider}\` has no default_model set — composer pill will show unknown.`,
          );
        }
      } catch (e: unknown) {
        console.warn("Failed to resolve admin default provider:", e);
      }
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
      console.warn("Failed to clear session model override:", e);
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

      // 4a. Pull the routable provider catalog in parallel with the
      //     session fetch. Used to reject stale `model_provider` values
      //     persisted by older builds before we touch `agent.state.model`.
      //     Non-fatal if it fails: downstream guards fail-open.
      const providersPromise = api
        .get<ProviderInfo[]>("/api/v1/chat/providers")
        .then((list) => {
          validProvidersRef.current = new Set(list.map((p) => p.id));
        })
        .catch((e: unknown) => {
          console.warn("Failed to load provider catalog for restore guard:", e);
        });

      // 4b. Resolve the active session key before creating the agent.
      //     Use the most recent existing session or create a new one.
      const existingSessions = await api.get<ChatSession[]>(
        "/api/v1/chat/sessions?limit=1&offset=0",
      );
      // Block on provider catalog here so the pre-mount restore step has
      // an authoritative allowlist. It's one cheap request; running it
      // serially after the sessions fetch keeps the code simple.
      await providersPromise;
      let initialSession: ChatSession;
      if (existingSessions.length > 0) {
        initialSession = existingSessions[0];
      } else {
        initialSession = await api.post<ChatSession>("/api/v1/chat/sessions", {});
      }
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

      // Restore the initial session's persisted model + thinking-level
      // BEFORE mounting the chat panel, so the composer pill reflects
      // the real selection and `onBeforeSend` does not see pi-agent-core's
      // "unknown" default as the first thing to persist.
      if (
        initialSession.model &&
        initialSession.model_provider &&
        isRoutableProvider(validProvidersRef.current, initialSession.model_provider)
      ) {
        agent.state.model = syntheticModel(
          initialSession.model_provider,
          initialSession.model,
        );
        lastPersistedRef.current = {
          model:    initialSession.model,
          provider: initialSession.model_provider,
          thinking: initialSession.thinking_level ?? null,
        };
      }
      if (initialSession.thinking_level) {
        agent.state.thinkingLevel = initialSession.thinking_level;
      }

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
          // The user just committed their first message — no more welcome.
          setShowWelcome(false);

          // Skip the PATCH when `agent.state.model` is pi-agent-core's
          // placeholder default (id/provider = "unknown"). Persisting it
          // would overwrite any previously saved rara provider with a
          // sentinel the kernel's DriverRegistry cannot route to, which
          // caused the original "LLM provider not configured" failure
          // (see #1554). `isUnknownModel` returns true for null/undefined
          // too, so the subsequent reads are safe without the `?.` guard.
          const picked = !isUnknownModel(agent.state.model);
          const model = picked ? agent.state.model!.id : null;
          const provider = picked ? agent.state.model!.provider : null;
          const thinking = asThinkingLevel(agent.state.thinkingLevel);

          // Nothing worth persisting.
          if (!model && !thinking) return;

          // Dedup consecutive identical writes — the chat UI round-trips
          // every send through this hook even when the selection hasn't
          // changed, and the PATCH wakes up the session index for nothing.
          const last = lastPersistedRef.current;
          if (last && last.model === model && last.provider === provider && last.thinking === thinking) {
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
    <div className="rara-chat relative flex h-screen w-screen flex-col">
      {/*
        Top utility bar — reserves its own row (not `absolute`) so the
        chat panel's message list can never render underneath the
        Sessions / Settings icons. Transparent + backdrop-blur so the
        chat-page canvas + radial glow read through as frosted glass.
      */}
      <div className="flex h-12 shrink-0 items-center gap-1 border-b border-border/40 bg-background/40 px-2 backdrop-blur-md">
        <button
          onClick={() => setShowSessionList(true)}
          className="flex h-9 w-9 items-center justify-center rounded-md text-muted-foreground hover:bg-secondary/60 hover:text-foreground transition-colors cursor-pointer"
          title="Sessions"
        >
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <path d="M3 12h18M3 6h18M3 18h18" />
          </svg>
        </button>
        {/*
          Opens the rara floating settings modal (provider keys, MCP
          servers, agent manifests, kernel config). Single source of
          truth since #1581 retired pi-mono's separate SettingsDialog.
        */}
        <button
          onClick={() => openSettings()}
          className="flex h-9 w-9 items-center justify-center rounded-md text-muted-foreground hover:bg-secondary/60 hover:text-foreground transition-colors cursor-pointer"
          title="Settings"
        >
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <circle cx="12" cy="12" r="3" />
            <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
          </svg>
        </button>
      </div>
      {/* Chat panel container — takes remaining vertical space. */}
      <div ref={containerRef} className="min-h-0 flex-1 w-full" />
      {/*
        Welcome overlay — rendered above pi-web-ui's empty message list
        when the active session has no messages. Pointer-events-none so
        clicks pass through to the composer below; flipped off the
        moment the user commits their first message (onBeforeSend).
      */}
      {showWelcome && !isInitializing && (
        <div className="pointer-events-none absolute inset-x-0 top-12 bottom-40 z-10 flex flex-col items-center justify-center gap-4 px-6 text-center">
          <h1 className="bg-gradient-to-br from-foreground via-foreground/85 to-foreground/50 bg-clip-text text-4xl font-semibold tracking-tight text-transparent sm:text-5xl">
            你好，我是 Rara
          </h1>
          <p className="max-w-md text-sm text-muted-foreground sm:text-base">
            有什么想聊的？写任务、问问题、让我帮你查点东西都行。
          </p>
        </div>
      )}
      {/*
        Voice button floats over pi-web-ui's composer, anchored to the
        viewport bottom-right so it sits on the same line as the send
        button without needing to patch pi-web-ui's input internals.
      */}
      <VoiceRecorder
        className="absolute bottom-[29px] left-14 z-20 !h-8 !w-8 !rounded-md !bg-transparent !shadow-none hover:!bg-accent"
        getSessionKey={() => agentRef.current?.sessionId}
        onComplete={reloadMessages}
      />
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
          onOpenAdmin={() => openSettings()}
        />
      )}
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
    </div>
  );
}
