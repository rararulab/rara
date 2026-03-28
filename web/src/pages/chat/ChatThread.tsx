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

import { useCallback, useEffect, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Bot,
  ChevronDown,
  ImagePlus,
  Link2,
  Loader2,
  Send,
  Trash2,
  X,
} from "lucide-react";
import type { CascadeStreamState } from "@/hooks/use-cascade";
import type {
  ChatMessageData,
  ChatSession,
} from "@/api/types";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import {
  buildOutboundChatContent,
  fileToImageBlock,
  imageBlockSrc,
  type ImageChatContentBlock,
} from "@/lib/chat-attachments";
import { useServerStatus } from "@/hooks/use-server-status";
import type { PendingDraft, StreamState, TurnMetrics, WebEvent } from "./types";
import { fetchMessages, updateSession } from "./api";
import { ConversationPanelToggleButton } from "./SessionSidebar";
import { MessageBubble } from "./MessageBubble";
import { ChangeModelDialog } from "./ModelPicker";
import { ActivityTree, StreamingBubble } from "./StreamingBubble";

// ---------------------------------------------------------------------------
// ChatThread (right panel) — SSE streaming version
// ---------------------------------------------------------------------------

const INITIAL_STREAM_STATE: StreamState = {
  isStreaming: false,
  text: "",
  reasoning: "",
  isThinking: false,
  activeTools: [],
  completedTools: [],
  turnRationale: "",
  error: null,
};

export function ChatThread({
  session,
  onClearMessages,
  panelCollapsed,
  onTogglePanel,
  initialDraft,
  onInitialDraftConsumed,
  onMessageClick,
  onStreamStateChange,
}: {
  session: ChatSession;
  onClearMessages: () => void;
  panelCollapsed: boolean;
  onTogglePanel: () => void;
  initialDraft?: PendingDraft | null;
  onInitialDraftConsumed?: () => void;
  onMessageClick?: (seq: number) => void;
  onStreamStateChange?: (isStreaming: boolean, state: CascadeStreamState) => void;
}) {
  const sessionKey = session.key;
  const queryClient = useQueryClient();
  const { isOnline } = useServerStatus();
  const [input, setInput] = useState("");
  const [attachments, setAttachments] = useState<ImageChatContentBlock[]>([]);
  const [imageInputVisible, setImageInputVisible] = useState(false);
  const [imageInputValue, setImageInputValue] = useState("");
  const [modelDialogOpen, setModelDialogOpen] = useState(false);
  const [stream, setStream] = useState<StreamState>(INITIAL_STREAM_STATE);
  const [latestMetrics, setLatestMetrics] = useState<TurnMetrics | null>(null);
  const wsRef = useRef<WebSocket | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // Notify parent of stream state changes for cascade viewer
  useEffect(() => {
    onStreamStateChange?.(stream.isStreaming, {
      reasoning: stream.reasoning,
      activeTools: stream.activeTools,
      completedTools: stream.completedTools,
    });
  }, [stream.isStreaming, stream.reasoning, stream.activeTools, stream.completedTools, onStreamStateChange]);

  const messagesQuery = useQuery({
    queryKey: ["chat-messages", sessionKey],
    queryFn: () => fetchMessages(sessionKey),
    enabled: !!sessionKey,
  });

  const messages = messagesQuery.data ?? [];

  const handleAddImageUrl = useCallback(() => {
    const url = imageInputValue.trim();
    if (!url) return;
    setAttachments((prev) => [...prev, { type: "image_url", url }]);
    setImageInputValue("");
    setImageInputVisible(false);
  }, [imageInputValue]);

  const handleRemoveImageUrl = useCallback((index: number) => {
    setAttachments((prev) => prev.filter((_, i) => i !== index));
  }, []);

  const handleFileSelection = useCallback(
    async (e: React.ChangeEvent<HTMLInputElement>) => {
      const inputEl = e.target;
      const files = Array.from(inputEl.files ?? []);
      inputEl.value = "";

      if (files.length === 0) return;

      try {
        const blocks = await Promise.all(files.map((file) => fileToImageBlock(file)));
        setAttachments((prev) => [...prev, ...blocks]);
      } catch {
        setStream((current) => ({
          ...current,
          error: "Failed to read image attachment",
        }));
      }
    },
    [],
  );

  const changeModelMutation = useMutation({
    mutationFn: (model: string) => updateSession(sessionKey, { model }),
    onSuccess: (_data, model) => {
      queryClient.setQueryData<ChatSession[]>(["chat-sessions"], (old) =>
        old?.map((s) => (s.key === sessionKey ? { ...s, model } : s)),
      );
    },
  });

  // WebSocket connection management
  // Uses a cleanedUp flag to handle React StrictMode double-mount gracefully,
  // and auto-reconnects on transient disconnects.
  useEffect(() => {
    if (!sessionKey) return;
    let cleanedUp = false;

    function connect() {
      if (cleanedUp) return;

      const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
      const baseUrl = import.meta.env.VITE_API_URL || "";
      const host = baseUrl ? new URL(baseUrl).host : window.location.host;
      const token = localStorage.getItem('access_token') ?? '';
      const url = `${protocol}//${host}/api/v1/kernel/chat/ws?session_key=${encodeURIComponent(sessionKey)}&token=${encodeURIComponent(token)}`;

      const ws = new WebSocket(url);
      wsRef.current = ws;

      ws.onopen = () => {
        if (cleanedUp) {
          ws.close();
          return;
        }
      };

      ws.onmessage = (e) => {
        if (cleanedUp) return;
        try {
          const event = JSON.parse(e.data) as WebEvent;
          switch (event.type) {
            case "text_delta":
              setStream((s) => ({ ...s, text: s.text + event.text }));
              break;
            case "reasoning_delta":
              setStream((s) => ({
                ...s,
                reasoning: s.reasoning + event.text,
              }));
              break;
            case "typing":
              setStream((s) => ({ ...s, isThinking: true }));
              break;
            case "turn_rationale":
              setStream((s) => ({
                ...s,
                turnRationale: event.text,
              }));
              break;
            case "tool_call_start":
              setStream((s) => ({
                ...s,
                activeTools: [
                  ...s.activeTools,
                  { id: event.id, name: event.name, arguments: event.arguments },
                ],
              }));
              break;
            case "tool_call_end": {
              setStream((s) => {
                const finished = s.activeTools.find((t) => t.id === event.id);
                return {
                  ...s,
                  activeTools: s.activeTools.filter((t) => t.id !== event.id),
                  completedTools: finished
                    ? [...s.completedTools, {
                        id: finished.id,
                        name: finished.name,
                        success: event.success,
                        result_preview: event.result_preview,
                        error: event.error,
                      }]
                    : s.completedTools,
                };
              });
              break;
            }
            case "progress":
              setStream((s) => ({
                ...s,
                isThinking: event.stage === "thinking",
              }));
              break;
            case "turn_metrics":
              setLatestMetrics({
                duration_ms: event.duration_ms,
                iterations: event.iterations,
                tool_calls: event.tool_calls,
                model: event.model,
              });
              break;
            case "done":
            case "message":
              setStream(INITIAL_STREAM_STATE);
              queryClient.invalidateQueries({
                queryKey: ["chat-messages", sessionKey],
              });
              queryClient.setQueryData<ChatSession[]>(
                ["chat-sessions"],
                (old) =>
                  old?.map((s) =>
                    s.key === sessionKey
                      ? {
                          ...s,
                          message_count: s.message_count + 2,
                          updated_at: new Date().toISOString(),
                        }
                      : s,
                  ),
              );
              break;
            case "error":
              setStream((s) => ({
                ...s,
                isStreaming: false,
                error: event.message,
              }));
              queryClient.invalidateQueries({
                queryKey: ["chat-messages", sessionKey],
              });
              break;
          }
        } catch {
          // Ignore non-JSON messages
        }
      };

      ws.onerror = () => {
        if (cleanedUp) return;
        setStream((s) => ({
          ...s,
          isStreaming: false,
          error: "WebSocket connection error",
        }));
      };

      ws.onclose = () => {
        // Only clear the ref if it still points to THIS WebSocket instance.
        // Prevents a stale onclose (from StrictMode's first mount) from
        // nullifying the ref set by the second mount's connect().
        if (wsRef.current === ws) {
          wsRef.current = null;
        }
        if (cleanedUp) return;
        // Auto-reconnect after delay
        setTimeout(() => connect(), 2000);
      };
    }

    connect();

    return () => {
      cleanedUp = true;
      if (wsRef.current) {
        wsRef.current.close();
        wsRef.current = null;
      }
    };
  }, [sessionKey, queryClient]);

  // WebSocket send
  const sendMessage = useCallback(
    (text: string, nextAttachments: ImageChatContentBlock[] = []) => {
      const trimmed = text.trim();
      if ((!trimmed && nextAttachments.length === 0) || stream.isStreaming || !isOnline) return;
      if (!wsRef.current || wsRef.current.readyState !== WebSocket.OPEN) return;

      setInput("");
      setAttachments([]);
      setImageInputVisible(false);
      setImageInputValue("");

      // Optimistically add user message to the cache
      const content = buildOutboundChatContent(trimmed, nextAttachments);
      const previous = queryClient.getQueryData<ChatMessageData[]>([
        "chat-messages",
        sessionKey,
      ]);
      const optimisticMsg: ChatMessageData = {
        seq: (previous?.length ?? 0) + 1,
        role: "user",
        content,
        created_at: new Date().toISOString(),
      };
      queryClient.setQueryData<ChatMessageData[]>(
        ["chat-messages", sessionKey],
        (old) => [...(old ?? []), optimisticMsg],
      );

      // Reset streaming state and send
      setStream({ ...INITIAL_STREAM_STATE, isStreaming: true });
      wsRef.current.send(
        typeof content === "string" ? content : JSON.stringify({ content }),
      );
    },
    [stream.isStreaming, isOnline, sessionKey, queryClient],
  );

  const handleSend = useCallback(() => {
    sendMessage(input, attachments);
  }, [attachments, input, sendMessage]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        handleSend();
      }
    },
    [handleSend],
  );

  const handleChangeModel = useCallback(
    (model: string) => {
      changeModelMutation.mutate(model);
    },
    [changeModelMutation],
  );

  // Auto-scroll to bottom (triggers on new messages or streaming text)
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages.length, stream.isStreaming, stream.text]);

  // Auto-focus textarea on session switch
  useEffect(() => {
    textareaRef.current?.focus();
  }, [sessionKey]);

  // Auto-resize textarea
  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 160)}px`;
  }, [input]);

  useEffect(() => {
    if (!initialDraft?.text) return;
    sendMessage(initialDraft.text);
    onInitialDraftConsumed?.();
  }, [initialDraft, onInitialDraftConsumed, sendMessage]);

  // Visible messages (exclude system)
  const visibleMessages = messages.filter((m) => m.role !== "system");

  // Extract short model name for display (e.g. "openai/gpt-4o" -> "gpt-4o")
  const modelDisplay = session.model
    ? session.model.split("/").pop() ?? session.model
    : "default";

  const isBusy = stream.isStreaming;

  return (
    <div className="relative flex min-w-0 flex-1 flex-col">
      {/* Thread header */}
      <div className="absolute inset-x-4 top-3 z-10 md:inset-x-8">
        <div className="grid grid-cols-[auto_1fr_auto] items-start gap-3">
          <div className="flex min-w-0 items-center gap-2">
            {panelCollapsed && (
              <ConversationPanelToggleButton
                collapsed
                onToggle={onTogglePanel}
              />
            )}
          </div>

          <div className="min-w-0 text-center">
            <p className="truncate text-xs font-medium text-muted-foreground/90">
              {messages.length} message{messages.length !== 1 ? "s" : ""} · {session.title ?? sessionKey}
            </p>
          </div>

          <div className="flex items-center justify-end gap-2">
            <button
              type="button"
              onClick={() => setModelDialogOpen(true)}
              title={session.model ?? "Click to select a model"}
              className="shrink-0"
            >
              <Badge
                variant="secondary"
                className="cursor-pointer gap-1 border-0 bg-background/50 text-xs shadow-none backdrop-blur hover:bg-background/75"
              >
                {modelDisplay}
                <ChevronDown className="h-3 w-3" />
              </Badge>
            </button>
            <Button
              variant="ghost"
              size="sm"
              className="rounded-lg bg-background/35 text-muted-foreground backdrop-blur hover:bg-background/70 hover:text-destructive"
              onClick={onClearMessages}
              title="Clear messages"
            >
              <Trash2 className="h-3.5 w-3.5" />
            </Button>
          </div>
        </div>
      </div>

      {/* Model change dialog */}
      <ChangeModelDialog
        open={modelDialogOpen}
        onOpenChange={setModelDialogOpen}
        currentModel={session.model ?? ""}
        onConfirm={handleChangeModel}
      />

      {/* Messages area */}
      <div className="flex-1 overflow-y-auto px-6 pb-40 pt-20 md:px-8 md:pb-44 md:pt-20">
        {messagesQuery.isLoading && (
          <div className="space-y-4">
            {Array.from({ length: 3 }).map((_, i) => (
              <div key={i} className="flex gap-3">
                <Skeleton className="h-8 w-8 rounded-full" />
                <Skeleton className="h-16 flex-1 rounded-xl" />
              </div>
            ))}
          </div>
        )}

        {!messagesQuery.isLoading && visibleMessages.length === 0 && !isBusy && (
          <div className="flex h-full flex-col items-center justify-center gap-3 text-muted-foreground">
            <Bot className="h-12 w-12 opacity-20" />
            <p className="text-sm opacity-80">
              Start a conversation by typing a message below.
            </p>
          </div>
        )}

        {!messagesQuery.isLoading && (
          <div className="space-y-4">
            {visibleMessages.map((msg, i) => {
              const isLastAssistant =
                msg.role === "assistant" &&
                !visibleMessages.slice(i + 1).some((m) => m.role === "assistant");
              return (
                <MessageBubble
                  key={msg.seq}
                  msg={msg}
                  metrics={isLastAssistant ? latestMetrics : undefined}
                  onClick={() => onMessageClick?.(msg.seq)}
                />
              );
            })}

            {/* Activity tree (real-time tool call trace) */}
            {(stream.activeTools.length > 0 ||
              stream.completedTools.length > 0) && (
              <ActivityTree stream={stream} />
            )}

            {/* Live streaming assistant bubble */}
            {(stream.isStreaming || stream.text || stream.error) && (
              <StreamingBubble stream={stream} />
            )}

            {/* Non-streaming error (e.g. connection failure before stream starts) */}
            {stream.error && !stream.isStreaming && !stream.text && (
              <div className="mx-auto max-w-md rounded-md border border-destructive/30 bg-destructive/10 px-4 py-2 text-center text-sm text-destructive">
                {stream.error}
              </div>
            )}

            <div ref={messagesEndRef} />
          </div>
        )}
      </div>

      {/* Input area */}
      <div className="pointer-events-none absolute inset-x-4 bottom-4 z-10 md:inset-x-8 md:bottom-6">
        <input
          ref={fileInputRef}
          type="file"
          accept="image/*"
          multiple
          className="hidden"
          onChange={(e) => {
            void handleFileSelection(e);
          }}
        />

        {/* Attached image previews */}
        {attachments.length > 0 && (
          <div className="pointer-events-auto mb-2 flex flex-wrap gap-2">
            {attachments.map((block, i) => (
              <div
                key={i}
                className="group relative h-16 w-16 overflow-hidden rounded-xl border border-input bg-muted shadow-sm"
              >
                <img
                  src={imageBlockSrc(block)}
                  alt=""
                  className="h-full w-full object-cover"
                  onError={(e) => {
                    (e.target as HTMLImageElement).style.display = "none";
                  }}
                />
                <button
                  type="button"
                  className="absolute -right-1 -top-1 flex h-5 w-5 items-center justify-center rounded-full bg-destructive text-destructive-foreground opacity-0 shadow-sm transition-opacity group-hover:opacity-100"
                  onClick={() => handleRemoveImageUrl(i)}
                  title="Remove image"
                >
                  <X className="h-3 w-3" />
                </button>
              </div>
            ))}
          </div>
        )}

        {/* Image URL input */}
        {imageInputVisible && (
          <div className="pointer-events-auto mb-2 flex items-center gap-2 rounded-2xl border border-border/40 bg-background/70 p-2 shadow-lg backdrop-blur">
            <input
              type="url"
              value={imageInputValue}
              onChange={(e) => setImageInputValue(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  handleAddImageUrl();
                }
                if (e.key === "Escape") {
                  setImageInputVisible(false);
                  setImageInputValue("");
                }
              }}
              placeholder="Paste image URL and press Enter..."
              className="flex-1 rounded-lg border border-input bg-background px-3 py-2 text-sm shadow-sm placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
              autoFocus
            />
            <Button
              variant="ghost"
              size="sm"
              className="rounded-lg"
              onClick={() => {
                setImageInputVisible(false);
                setImageInputValue("");
              }}
            >
              Cancel
            </Button>
          </div>
        )}

        <div className="pointer-events-auto flex items-end gap-2 rounded-2xl border border-border/40 bg-background/70 p-2 shadow-[0_10px_40px_rgba(15,23,42,0.12)] backdrop-blur-xl">
          <Button
            variant="ghost"
            size="icon"
            className="h-10 w-10 shrink-0 rounded-xl text-muted-foreground hover:bg-background/70 hover:text-foreground"
            onClick={() => fileInputRef.current?.click()}
            disabled={isBusy || !isOnline}
            title="Upload image"
          >
            <ImagePlus className="h-4 w-4" />
          </Button>
          <Button
            variant="ghost"
            size="icon"
            className="h-10 w-10 shrink-0 rounded-xl text-muted-foreground hover:bg-background/70 hover:text-foreground"
            onClick={() => setImageInputVisible((v) => !v)}
            disabled={isBusy || !isOnline}
            title="Attach image URL"
          >
            <Link2 className="h-4 w-4" />
          </Button>
          <textarea
            ref={textareaRef}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={isOnline ? "Type a message... (Enter to send, Shift+Enter for newline)" : "Server offline -- sending disabled"}
            rows={1}
            disabled={isBusy || !isOnline}
            autoFocus
            className="flex-1 resize-none appearance-none border-0 bg-transparent px-2 py-2.5 text-sm text-foreground shadow-none placeholder:text-muted-foreground focus:outline-none focus:ring-0 focus-visible:outline-none focus-visible:ring-0 disabled:cursor-not-allowed disabled:opacity-50"
          />
          <Button
            size="icon"
            className="h-10 w-10 shrink-0 rounded-xl shadow-sm"
            onClick={handleSend}
            disabled={(!input.trim() && attachments.length === 0) || isBusy || !isOnline}
            title={isOnline ? "Send message" : "Server offline"}
          >
            {isBusy ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <Send className="h-4 w-4" />
            )}
          </Button>
        </div>
      </div>
    </div>
  );
}
