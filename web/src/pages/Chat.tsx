/*
 * Copyright 2025 Crrow
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
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import {
  Bot,
  ImagePlus,
  Loader2,
  MessageSquarePlus,
  PanelLeftClose,
  PanelLeftOpen,
  Send,
  Trash2,
  User,
  X,
} from "lucide-react";
import { api } from "@/api/client";
import type {
  ChatMessageData,
  ChatSession,
  SendMessageResponse,
} from "@/api/types";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";
import { useServerStatus } from "@/hooks/use-server-status";

// ---------------------------------------------------------------------------
// API helpers
// ---------------------------------------------------------------------------

function fetchSessions() {
  return api.get<ChatSession[]>("/api/v1/chat/sessions?limit=100&offset=0");
}

function fetchMessages(key: string) {
  return api.get<ChatMessageData[]>(
    `/api/v1/chat/sessions/${encodeURIComponent(key)}/messages?limit=200`,
  );
}

function createSession(body: {
  key: string;
  title?: string;
  model?: string;
  system_prompt?: string;
}) {
  return api.post<ChatSession>("/api/v1/chat/sessions", body);
}

function sendMessage(key: string, text: string, imageUrls?: string[]) {
  const body: { text: string; image_urls?: string[] } = { text };
  if (imageUrls && imageUrls.length > 0) {
    body.image_urls = imageUrls;
  }
  return api.post<SendMessageResponse>(
    `/api/v1/chat/sessions/${encodeURIComponent(key)}/send`,
    body,
  );
}

function deleteSession(key: string) {
  return api.del<void>(
    `/api/v1/chat/sessions/${encodeURIComponent(key)}`,
  );
}

function clearMessages(key: string) {
  return api.del<void>(
    `/api/v1/chat/sessions/${encodeURIComponent(key)}/messages`,
  );
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function generateKey(): string {
  return `chat-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}

function extractTextContent(content: ChatMessageData["content"]): string {
  if (typeof content === "string") return content;
  return content
    .filter((b): b is { type: "text"; text: string } => b.type === "text")
    .map((b) => b.text)
    .join("");
}

function formatTime(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const now = new Date();
  if (d.toDateString() === now.toDateString()) {
    return d.toLocaleTimeString(undefined, {
      hour: "2-digit",
      minute: "2-digit",
    });
  }
  return d.toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}

// ---------------------------------------------------------------------------
// SessionList (left panel)
// ---------------------------------------------------------------------------

function SessionList({
  sessions,
  activeKey,
  onSelect,
  onCreate,
  onDelete,
  isLoading,
  collapsed,
  onToggleCollapse,
}: {
  sessions: ChatSession[];
  activeKey: string | null;
  onSelect: (key: string) => void;
  onCreate: () => void;
  onDelete: (key: string) => void;
  isLoading: boolean;
  collapsed: boolean;
  onToggleCollapse: () => void;
}) {
  return (
    <div
      className={cn(
        "flex flex-col border-r bg-card transition-all duration-200",
        collapsed ? "w-12" : "w-64",
      )}
    >
      {/* Header */}
      <div
        className={cn(
          "flex items-center border-b",
          collapsed ? "justify-center p-2" : "justify-between px-3 py-2",
        )}
      >
        {!collapsed && (
          <h2 className="text-sm font-semibold truncate">Conversations</h2>
        )}
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7 shrink-0"
          onClick={onToggleCollapse}
          title={collapsed ? "Expand panel" : "Collapse panel"}
        >
          {collapsed ? (
            <PanelLeftOpen className="h-4 w-4" />
          ) : (
            <PanelLeftClose className="h-4 w-4" />
          )}
        </Button>
      </div>

      {/* New chat button */}
      <div className={cn("border-b", collapsed ? "p-1" : "p-2")}>
        <Button
          variant="outline"
          size={collapsed ? "icon" : "sm"}
          className={cn("shrink-0", collapsed ? "mx-auto h-8 w-8" : "w-full")}
          onClick={onCreate}
          title="New conversation"
        >
          <MessageSquarePlus className="h-4 w-4" />
          {!collapsed && <span className="ml-1.5">New Chat</span>}
        </Button>
      </div>

      {/* Session list */}
      <div className="flex-1 overflow-y-auto">
        {isLoading && (
          <div className={cn("space-y-2", collapsed ? "p-1" : "p-2")}>
            {Array.from({ length: 4 }).map((_, i) => (
              <Skeleton
                key={i}
                className={cn(collapsed ? "h-8 w-8 mx-auto" : "h-14 w-full")}
              />
            ))}
          </div>
        )}
        {!isLoading && sessions.length === 0 && !collapsed && (
          <div className="p-4 text-center text-xs text-muted-foreground">
            No conversations yet.
            <br />
            Click &quot;New Chat&quot; to start.
          </div>
        )}
        {!isLoading && (
          <div className={cn("space-y-0.5", collapsed ? "p-1" : "p-2")}>
            {sessions.map((s) => (
              <button
                key={s.key}
                type="button"
                title={collapsed ? (s.title ?? s.key) : undefined}
                className={cn(
                  "group relative flex w-full items-center rounded-md text-left text-sm transition-colors",
                  collapsed
                    ? "justify-center p-2"
                    : "gap-2 px-2.5 py-2",
                  activeKey === s.key
                    ? "bg-accent text-accent-foreground"
                    : "text-muted-foreground hover:bg-accent/50 hover:text-accent-foreground",
                )}
                onClick={() => onSelect(s.key)}
              >
                <Bot className="h-4 w-4 shrink-0" />
                {!collapsed && (
                  <>
                    <div className="min-w-0 flex-1">
                      <p className="truncate text-sm font-medium">
                        {s.title ?? s.key}
                      </p>
                      {s.preview && (
                        <p className="truncate text-xs text-muted-foreground">
                          {s.preview}
                        </p>
                      )}
                    </div>
                    <span className="shrink-0 text-[10px] text-muted-foreground">
                      {formatTime(s.updated_at)}
                    </span>
                    <button
                      type="button"
                      className="absolute right-1 top-1 hidden rounded p-0.5 text-muted-foreground hover:text-destructive group-hover:block"
                      onClick={(e) => {
                        e.stopPropagation();
                        onDelete(s.key);
                      }}
                      title="Delete conversation"
                    >
                      <Trash2 className="h-3 w-3" />
                    </button>
                  </>
                )}
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// MessageBubble
// ---------------------------------------------------------------------------

function ImageBlock({ url }: { url: string }) {
  const [failed, setFailed] = useState(false);

  if (failed) {
    return (
      <div className="flex h-32 w-48 items-center justify-center rounded-lg border border-dashed border-muted-foreground/30 bg-muted/30 text-xs text-muted-foreground">
        Image failed to load
      </div>
    );
  }

  return (
    <img
      src={url}
      alt=""
      className="max-h-64 max-w-xs rounded-lg object-contain"
      onError={() => setFailed(true)}
    />
  );
}

function MessageBubble({ msg }: { msg: ChatMessageData }) {
  const isUser = msg.role === "user";
  const isSystem = msg.role === "system";
  const isMultimodal = Array.isArray(msg.content);
  const text = extractTextContent(msg.content);

  if (isSystem) {
    return (
      <div className="mx-auto max-w-md rounded-md bg-muted/50 px-4 py-2 text-center text-xs text-muted-foreground italic">
        {text}
      </div>
    );
  }

  return (
    <div
      className={cn("flex gap-3", isUser ? "flex-row-reverse" : "flex-row")}
    >
      {/* Avatar */}
      <div
        className={cn(
          "flex h-8 w-8 shrink-0 items-center justify-center rounded-full text-xs font-medium",
          isUser
            ? "bg-primary text-primary-foreground"
            : "bg-muted text-muted-foreground",
        )}
      >
        {isUser ? <User className="h-4 w-4" /> : <Bot className="h-4 w-4" />}
      </div>

      {/* Content */}
      <div
        className={cn(
          "max-w-[75%] rounded-xl px-4 py-2.5",
          isUser
            ? "bg-primary text-primary-foreground"
            : "bg-muted text-foreground",
        )}
      >
        {isMultimodal ? (
          <div className="space-y-2">
            {(msg.content as import("@/api/types").ChatContentBlock[]).map(
              (block, i) => {
                if (block.type === "text") {
                  return isUser ? (
                    <p key={i} className="whitespace-pre-wrap text-sm">
                      {block.text}
                    </p>
                  ) : (
                    <div
                      key={i}
                      className="prose prose-sm dark:prose-invert max-w-none [&_pre]:overflow-x-auto [&_pre]:rounded-md [&_pre]:bg-background/50 [&_pre]:p-3 [&_code]:rounded [&_code]:bg-background/50 [&_code]:px-1 [&_code]:py-0.5 [&_code]:text-xs"
                    >
                      <ReactMarkdown remarkPlugins={[remarkGfm]}>
                        {block.text}
                      </ReactMarkdown>
                    </div>
                  );
                }
                if (block.type === "image_url") {
                  return <ImageBlock key={i} url={block.url} />;
                }
                return null;
              },
            )}
          </div>
        ) : isUser ? (
          <p className="whitespace-pre-wrap text-sm">{text}</p>
        ) : (
          <div className="prose prose-sm dark:prose-invert max-w-none [&_pre]:overflow-x-auto [&_pre]:rounded-md [&_pre]:bg-background/50 [&_pre]:p-3 [&_code]:rounded [&_code]:bg-background/50 [&_code]:px-1 [&_code]:py-0.5 [&_code]:text-xs">
            <ReactMarkdown remarkPlugins={[remarkGfm]}>{text}</ReactMarkdown>
          </div>
        )}
        <p
          className={cn(
            "mt-1 text-[10px]",
            isUser ? "text-primary-foreground/60" : "text-muted-foreground",
          )}
        >
          {formatTime(msg.created_at)}
        </p>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// ChatThread (right panel)
// ---------------------------------------------------------------------------

function ChatThread({
  sessionKey,
  onClearMessages,
}: {
  sessionKey: string;
  onClearMessages: () => void;
}) {
  const queryClient = useQueryClient();
  const { isOnline } = useServerStatus();
  const [input, setInput] = useState("");
  const [imageUrls, setImageUrls] = useState<string[]>([]);
  const [imageInputVisible, setImageInputVisible] = useState(false);
  const [imageInputValue, setImageInputValue] = useState("");
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const messagesQuery = useQuery({
    queryKey: ["chat-messages", sessionKey],
    queryFn: () => fetchMessages(sessionKey),
    enabled: !!sessionKey,
  });

  const messages = messagesQuery.data ?? [];

  const sendMutation = useMutation({
    mutationFn: (vars: { text: string; imageUrls?: string[] }) =>
      sendMessage(sessionKey, vars.text, vars.imageUrls),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ["chat-messages", sessionKey],
      });
      queryClient.invalidateQueries({ queryKey: ["chat-sessions"] });
    },
  });

  const handleAddImageUrl = useCallback(() => {
    const url = imageInputValue.trim();
    if (!url) return;
    setImageUrls((prev) => [...prev, url]);
    setImageInputValue("");
    setImageInputVisible(false);
  }, [imageInputValue]);

  const handleRemoveImageUrl = useCallback((index: number) => {
    setImageUrls((prev) => prev.filter((_, i) => i !== index));
  }, []);

  const handleSend = useCallback(() => {
    const text = input.trim();
    if (!text || sendMutation.isPending || !isOnline) return;
    const urls = imageUrls.length > 0 ? [...imageUrls] : undefined;
    setInput("");
    setImageUrls([]);
    setImageInputVisible(false);
    setImageInputValue("");
    sendMutation.mutate({ text, imageUrls: urls });
  }, [input, imageUrls, sendMutation, isOnline]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        handleSend();
      }
    },
    [handleSend],
  );

  // Auto-scroll to bottom
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages.length, sendMutation.isPending]);

  // Auto-resize textarea
  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 160)}px`;
  }, [input]);

  // Visible messages (exclude system)
  const visibleMessages = messages.filter((m) => m.role !== "system");

  return (
    <div className="flex flex-1 flex-col">
      {/* Thread header */}
      <div className="flex items-center justify-between border-b px-4 py-2.5">
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-semibold">{sessionKey}</p>
          <p className="text-xs text-muted-foreground">
            {messages.length} message{messages.length !== 1 ? "s" : ""}
          </p>
        </div>
        <Button
          variant="ghost"
          size="sm"
          className="text-muted-foreground hover:text-destructive"
          onClick={onClearMessages}
          title="Clear messages"
        >
          <Trash2 className="h-3.5 w-3.5" />
        </Button>
      </div>

      {/* Messages area */}
      <div className="flex-1 overflow-y-auto px-4 py-4">
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

        {!messagesQuery.isLoading && visibleMessages.length === 0 && (
          <div className="flex h-full flex-col items-center justify-center gap-3 text-muted-foreground">
            <Bot className="h-12 w-12 opacity-30" />
            <p className="text-sm">
              Start a conversation by typing a message below.
            </p>
          </div>
        )}

        {!messagesQuery.isLoading && (
          <div className="space-y-4">
            {visibleMessages.map((msg) => (
              <MessageBubble key={msg.seq} msg={msg} />
            ))}

            {/* Pending assistant response indicator */}
            {sendMutation.isPending && (
              <div className="flex gap-3">
                <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-muted text-muted-foreground">
                  <Bot className="h-4 w-4" />
                </div>
                <div className="flex items-center gap-2 rounded-xl bg-muted px-4 py-2.5">
                  <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
                  <span className="text-sm text-muted-foreground">
                    Thinking...
                  </span>
                </div>
              </div>
            )}

            {/* Error display */}
            {sendMutation.isError && (
              <div className="mx-auto max-w-md rounded-md border border-destructive/30 bg-destructive/10 px-4 py-2 text-center text-sm text-destructive">
                {sendMutation.error instanceof Error
                  ? sendMutation.error.message
                  : "Failed to send message. Please try again."}
              </div>
            )}

            <div ref={messagesEndRef} />
          </div>
        )}
      </div>

      {/* Input area */}
      <div className="border-t bg-card px-4 py-3">
        {!isOnline && (
          <p className="mb-2 text-center text-xs text-destructive">
            Server is offline. Sending is disabled until the connection is restored.
          </p>
        )}

        {/* Attached image previews */}
        {imageUrls.length > 0 && (
          <div className="mb-2 flex flex-wrap gap-2">
            {imageUrls.map((url, i) => (
              <div
                key={i}
                className="group relative h-16 w-16 overflow-hidden rounded-lg border border-input bg-muted"
              >
                <img
                  src={url}
                  alt=""
                  className="h-full w-full object-cover"
                  onError={(e) => {
                    (e.target as HTMLImageElement).style.display = "none";
                  }}
                />
                <button
                  type="button"
                  className="absolute -right-1 -top-1 flex h-5 w-5 items-center justify-center rounded-full bg-destructive text-destructive-foreground opacity-0 transition-opacity group-hover:opacity-100"
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
          <div className="mb-2 flex items-center gap-2">
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
              className="flex-1 rounded-md border border-input bg-background px-3 py-1.5 text-sm shadow-sm placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
              autoFocus
            />
            <Button
              variant="ghost"
              size="sm"
              onClick={() => {
                setImageInputVisible(false);
                setImageInputValue("");
              }}
            >
              Cancel
            </Button>
          </div>
        )}

        <div className="flex items-end gap-2">
          <Button
            variant="ghost"
            size="icon"
            className="h-10 w-10 shrink-0 text-muted-foreground hover:text-foreground"
            onClick={() => setImageInputVisible((v) => !v)}
            disabled={sendMutation.isPending || !isOnline}
            title="Attach image URL"
          >
            <ImagePlus className="h-4 w-4" />
          </Button>
          <textarea
            ref={textareaRef}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={isOnline ? "Type a message... (Enter to send, Shift+Enter for newline)" : "Server offline -- sending disabled"}
            rows={1}
            disabled={sendMutation.isPending || !isOnline}
            className="flex-1 resize-none rounded-lg border border-input bg-background px-3 py-2.5 text-sm shadow-sm placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50"
          />
          <Button
            size="icon"
            className="h-10 w-10 shrink-0"
            onClick={handleSend}
            disabled={!input.trim() || sendMutation.isPending || !isOnline}
            title={isOnline ? "Send message" : "Server offline"}
          >
            {sendMutation.isPending ? (
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

// ---------------------------------------------------------------------------
// EmptyState (when no session is selected)
// ---------------------------------------------------------------------------

function EmptyState({ onCreate }: { onCreate: () => void }) {
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-4 text-muted-foreground">
      <Bot className="h-16 w-16 opacity-20" />
      <div className="text-center">
        <p className="text-lg font-medium text-foreground">
          Welcome to Chat
        </p>
        <p className="mt-1 text-sm">
          Select a conversation from the sidebar or start a new one.
        </p>
      </div>
      <Button onClick={onCreate}>
        <MessageSquarePlus className="h-4 w-4" />
        New Conversation
      </Button>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Chat (main page component)
// ---------------------------------------------------------------------------

export default function Chat() {
  const queryClient = useQueryClient();
  const [activeKey, setActiveKey] = useState<string | null>(null);
  const [panelCollapsed, setPanelCollapsed] = useState(false);

  const sessionsQuery = useQuery({
    queryKey: ["chat-sessions"],
    queryFn: fetchSessions,
  });

  const sessions = sessionsQuery.data ?? [];

  const createMutation = useMutation({
    mutationFn: createSession,
    onSuccess: (session) => {
      queryClient.invalidateQueries({ queryKey: ["chat-sessions"] });
      setActiveKey(session.key);
    },
  });

  const deleteMutation = useMutation({
    mutationFn: deleteSession,
    onSuccess: (_data, deletedKey) => {
      queryClient.invalidateQueries({ queryKey: ["chat-sessions"] });
      queryClient.removeQueries({
        queryKey: ["chat-messages", deletedKey],
      });
      if (activeKey === deletedKey) {
        setActiveKey(null);
      }
    },
  });

  const clearMutation = useMutation({
    mutationFn: clearMessages,
    onSuccess: (_data, clearedKey) => {
      queryClient.invalidateQueries({
        queryKey: ["chat-messages", clearedKey],
      });
      queryClient.invalidateQueries({ queryKey: ["chat-sessions"] });
    },
  });

  const handleCreate = useCallback(() => {
    const key = generateKey();
    createMutation.mutate({
      key,
      title: `Chat ${new Date().toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" })}`,
    });
  }, [createMutation]);

  const handleDelete = useCallback(
    (key: string) => {
      deleteMutation.mutate(key);
    },
    [deleteMutation],
  );

  const handleClearMessages = useCallback(() => {
    if (!activeKey) return;
    clearMutation.mutate(activeKey);
  }, [activeKey, clearMutation]);

  return (
    <div className="flex h-full">
      {/* Left panel: session list */}
      <SessionList
        sessions={sessions}
        activeKey={activeKey}
        onSelect={setActiveKey}
        onCreate={handleCreate}
        onDelete={handleDelete}
        isLoading={sessionsQuery.isLoading}
        collapsed={panelCollapsed}
        onToggleCollapse={() => setPanelCollapsed((p) => !p)}
      />

      {/* Right panel: chat thread or empty state */}
      {activeKey ? (
        <ChatThread
          key={activeKey}
          sessionKey={activeKey}
          onClearMessages={handleClearMessages}
        />
      ) : (
        <EmptyState onCreate={handleCreate} />
      )}
    </div>
  );
}
